mod cache;
mod cli;
mod config;
mod doctor;
mod error;
mod installer;
mod manifest;
mod paths;
mod resolve;
mod shim;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use clap::Parser;
use semver::Version;

use crate::cache::Cache;
use crate::cli::{Cli, Command, ConfigCommand, ShimCommand};
use crate::config::{Config, Scope};
use crate::doctor::run_doctor;
use crate::error::AppError;
use crate::installer::install;
use crate::manifest::{Manifest, ManifestStore, Target};
use crate::paths::{cache_dir, normalize_path, shims_dir};
use crate::resolve::{resolve_tool, ResolutionSource};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(err.exit_code());
    }
}

fn run() -> Result<(), AppError> {
    if let Some(tool) = std::env::var_os(shim::SHIM_TOOL_ENV_VAR) {
        if let Some(tool) = tool.to_str().filter(|value| !value.is_empty()) {
            return shim::run_as_shim(tool);
        }
    }

    let argv0 = std::env::args().next().unwrap_or_default();
    let exec_name = std::path::Path::new(&argv0)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("ampland");
    let exec_lower = exec_name.to_ascii_lowercase();
    if exec_lower != "ampland" && !exec_lower.starts_with("ampland-") {
        return shim::run_as_shim(exec_name);
    }

    let cli = Cli::parse();
    let (mut config, config_path) = Config::load(cli.config.as_deref())?;
    let cache_root = cli.cache_dir.unwrap_or(cache_dir()?);
    let shims_root = cli.shims_dir.clone().unwrap_or(shims_dir()?);
    let cache = Cache::new(cache_root.clone());

    match cli.command {
        Command::Use {
            tool,
            version,
            global,
            path,
        } => {
            let target = Target::current()?;
            let manifest = ManifestStore::new(&cache_root, &config.manifest).load()?;
            let cwd = resolve_path(cli.path.clone(), path.clone())?;

            // If tool is None, read from .tool-versions
            let tools_to_set = if let Some(tool) = tool {
                let (tool, version) = normalize_tool_version_arg(tool, version);
                let Some(version) = version else {
                    return Err(AppError::Config {
                        message: "use requires a version (e.g. ampland use node 22 or node@22)"
                            .to_string(),
                    });
                };
                vec![(tool, version)]
            } else {
                let tool_versions_path = cwd.join(".tool-versions");
                if !tool_versions_path.exists() {
                    return Err(AppError::Config {
                        message: format!(
                            "no .tool-versions file found at {}",
                            tool_versions_path.display()
                        ),
                    });
                }
                parse_tool_versions_file(&tool_versions_path)?
            };

            let mut scope_label = None;
            let mut installed_tools = Vec::new();

            for (tool, version_spec) in tools_to_set {
                let version = resolve_version_spec(&manifest, &tool, &version_spec, &target)?;

                if global {
                    config.global.tools.insert(tool.clone(), version.clone());
                } else {
                    let pattern = normalize_scope_pattern(&cwd);
                    upsert_scope_tool(&mut config, &pattern, &tool, &version);
                    scope_label = Some(pattern);
                }

                if !cache.is_installed(&tool, &version) {
                    let package = manifest.resolve(&tool, &version, &target).ok_or_else(|| {
                        AppError::Cache {
                            message: format!(
                                "no installer for {tool}@{version} ({}/{})",
                                target.platform, target.arch
                            ),
                        }
                    })?;
                    let bin_path = install(&cache, &tool, &version, &package)?;
                    if !cli.quiet {
                        println!("installed {tool}@{version} -> {}", bin_path.display());
                    }
                }
                installed_tools.push((tool, version));
            }

            config.save(&config_path)?;
            shim::rebuild_shims(&config, &cache_root, cli.shims_dir.as_deref())?;

            if !cli.quiet {
                for (tool, version) in installed_tools {
                    if global {
                        println!("set {tool}@{version} for global");
                    } else if let Some(ref pattern) = scope_label {
                        println!("set {tool}@{version} for {pattern}");
                    }
                }
            }
        }
        Command::Install { tool, version } => {
            let (tool, version) = normalize_tool_version_arg(tool, version);
            let target = Target::current()?;
            let manifest = ManifestStore::new(&cache_root, &config.manifest).load()?;
            let version = match version {
                Some(version) => resolve_version_spec(&manifest, &tool, &version, &target)?,
                None => resolve_latest_version(&manifest, &tool, &target)?,
            };
            let package =
                manifest
                    .resolve(&tool, &version, &target)
                    .ok_or_else(|| AppError::Cache {
                        message: format!(
                            "no installer for {tool}@{version} ({}/{})",
                            target.platform, target.arch
                        ),
                    })?;
            let bin_path = install(&cache, &tool, &version, &package)?;
            if !cli.quiet {
                println!("installed {tool}@{version} -> {}", bin_path.display());
            }
        }
        Command::Uninstall { tool, version } => {
            let (tool, version) = normalize_tool_version_arg(tool, version);
            let Some(version) = version else {
                return Err(AppError::Config {
                    message:
                        "uninstall requires a version (e.g. ampland uninstall node 22 or node@22)"
                            .to_string(),
                });
            };
            let cwd = resolve_path(cli.path.clone(), None)?;
            let mut usages = config.is_tool_version_in_use(&tool, &version);
            let mut removed_from_current_scope = false;
            if !usages.is_empty() {
                if let Ok(resolution) = resolve_tool(&config, &cwd, &tool) {
                    if let ResolutionSource::Scope { pattern } = resolution.source {
                        if resolution.version == version
                            && config.remove_tool_version_from_scope(&pattern, &tool, &version)?
                        {
                            removed_from_current_scope = true;
                            config.save(&config_path)?;
                            usages = config.is_tool_version_in_use(&tool, &version);
                        }
                    }
                }
            }
            if !usages.is_empty() {
                if removed_from_current_scope {
                    if !cli.quiet {
                        println!(
                            "removed config for {tool}@{version}; still used elsewhere, cache kept"
                        );
                    }
                    return Ok(());
                }
                return Err(AppError::Config {
                    message: format!(
                        "{tool}@{version} is still in use. Configurations found in: {}",
                        usages.join(", ")
                    ),
                });
            }
            cache.uninstall(&tool, &version)?;
            let tool_still_configured = config.all_tool_versions().contains_key(&tool);
            if !tool_still_configured {
                shim::rebuild_shims(&config, &cache_root, cli.shims_dir.as_deref())?;
            }
            if !cli.quiet {
                println!("removed {tool}@{version}");
            }
        }
        Command::Search { query } => {
            let target = Target::current()?;
            let manifest = ManifestStore::new(&cache_root, &config.manifest).load()?;
            let needle = query.map(|value| value.to_lowercase());
            let mut results: BTreeMap<String, Vec<String>> = BTreeMap::new();

            for tool in &manifest.tools {
                if let Some(value) = &needle {
                    if !tool.name.to_lowercase().contains(value) {
                        continue;
                    }
                }

                let mut versions: Vec<String> = tool
                    .versions
                    .iter()
                    .filter(|entry| entry.platform == target.platform && entry.arch == target.arch)
                    .map(|entry| entry.ver.clone())
                    .collect();
                if versions.is_empty() {
                    continue;
                }
                versions.sort();
                versions.dedup();
                results.insert(tool.name.clone(), versions);
            }

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else if !cli.quiet {
                for (tool, versions) in results {
                    println!("{tool}: {}", versions.join(", "));
                }
            }
        }
        Command::List => {
            let installed = cache.list_installed()?;
            if cli.json {
                let mut output = BTreeMap::new();
                for (tool, versions) in installed {
                    output.insert(tool, versions);
                }
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else if !cli.quiet {
                for (tool, versions) in installed {
                    println!("{tool}: {}", versions.join(", "));
                }
            }
        }
        Command::Gc => {
            let keep = config.all_tool_versions();
            let removed = cache.gc(&keep)?;
            if cli.json {
                let display: Vec<String> = removed
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect();
                println!("{}", serde_json::to_string_pretty(&display)?);
            } else if !cli.quiet {
                for path in removed {
                    println!("removed {}", path.display());
                }
            }
        }
        Command::Doctor => {
            let cwd = resolve_path(cli.path, None)?;
            let report = run_doctor(&config, &cwd, &config_path, &cache_root, &shims_root)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if !cli.quiet {
                println!("config: {}", report.config_path.display());
                println!("cache: {}", report.cache_root.display());
                println!("shims: {}", report.shims_root.display());
                println!("shims in PATH: {}", report.shims_in_path);
                if !report.conflicts.is_empty() {
                    println!("conflicts:");
                    for conflict in report.conflicts {
                        println!("  {}", conflict.display());
                    }
                }
                if !report.missing_installs.is_empty() {
                    println!("missing installs:");
                    for item in report.missing_installs {
                        println!("  {item}");
                    }
                }
            }
        }
        Command::Which { tool } => {
            let cwd = resolve_path(cli.path, None)?;
            let target = Target::current()?;
            let manifest = ManifestStore::new(&cache_root, &config.manifest).load()?;
            let resolution =
                shim::resolve_bin_path(&config, &cwd, &tool, &cache, &manifest, &target)?;
            if !cli.quiet {
                println!("{}", resolution.path.display());
            }
        }
        Command::Explain { tool } => {
            let cwd = resolve_path(cli.path, None)?;
            let resolution = resolve_tool(&config, &cwd, &tool)?;
            if cli.json {
                let mut output = BTreeMap::new();
                output.insert("tool", resolution.tool.clone());
                output.insert("version", resolution.version.clone());
                let source = match &resolution.source {
                    ResolutionSource::Global => "global".to_string(),
                    ResolutionSource::Scope { pattern } => format!("scope:{pattern}"),
                    ResolutionSource::ScopedFallback { pattern } => {
                        format!("scope-fallback:{pattern}")
                    }
                };
                output.insert("source", source);
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else if !cli.quiet {
                println!("{tool}@{}", resolution.version);
                match resolution.source {
                    ResolutionSource::Global => println!("source: global"),
                    ResolutionSource::Scope { pattern } => println!("source: scope {pattern}"),
                    ResolutionSource::ScopedFallback { pattern } => {
                        println!("source: scope {pattern} (fallback to global)")
                    }
                }
            }
        }
        Command::Activate => {
            if !cli.quiet {
                let shell = detect_shell_kind();
                let shims_value = shims_root.to_string_lossy();
                match shell {
                    ShellKind::Posix => {
                        let value = escape_for_double_quotes(&shims_value);
                        println!("export PATH=\"{}:$PATH\"", value);
                    }
                    ShellKind::PowerShell => {
                        let value = escape_for_powershell_double_quotes(&shims_value);
                        let separator = if cfg!(windows) { ";" } else { ":" };
                        println!("$env:PATH = \"{}{}$env:PATH\"", value, separator);
                    }
                    ShellKind::Cmd => {
                        let value = escape_for_cmd_set(&shims_value);
                        println!("set \"PATH={};%PATH%\"", value);
                    }
                }
            }
        }
        Command::Shim { command } => match command {
            ShimCommand::Rebuild => {
                let created = shim::rebuild_shims(&config, &cache_root, cli.shims_dir.as_deref())?;
                if !cli.quiet {
                    for path in created {
                        println!("shimmed {}", path.display());
                    }
                }
            }
            ShimCommand::Add { tool } => {
                let path = shim::add_shim(&tool, cli.shims_dir.as_deref())?;
                if !cli.quiet {
                    println!("shimmed {}", path.display());
                }
            }
        },
        Command::Config { command } => match command {
            ConfigCommand::Show => {
                if cli.json {
                    let mut output = std::collections::BTreeMap::new();
                    output.insert("path", config_path.to_string_lossy().to_string());
                    let contents = if config_path.exists() {
                        std::fs::read_to_string(&config_path)?
                    } else {
                        String::new()
                    };
                    output.insert("contents", contents);
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else if !cli.quiet {
                    println!("config: {}", config_path.display());
                    if config_path.exists() {
                        let contents = std::fs::read_to_string(&config_path)?;
                        println!("{}", contents);
                    } else {
                        println!("(file does not exist)");
                    }
                }
            }
            ConfigCommand::Edit => {
                // Ensure the config file and its parent directories exist
                if let Some(parent) = config_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                if !config_path.exists() {
                    std::fs::write(&config_path, "")?;
                }

                // Open in editor
                let editor = std::env::var("VISUAL")
                    .or_else(|_| std::env::var("EDITOR"))
                    .unwrap_or_else(|_| {
                        if cfg!(windows) {
                            "notepad".to_string()
                        } else {
                            "vi".to_string()
                        }
                    });

                let status = std::process::Command::new(&editor)
                    .arg(&config_path)
                    .status()?;

                if !status.success() {
                    return Err(AppError::Config {
                        message: format!("editor {} exited with error", editor),
                    });
                }

                if !cli.quiet {
                    println!("edited {}", config_path.display());
                }
            }
        },
    }

    Ok(())
}

fn resolve_path(global: Option<PathBuf>, command: Option<PathBuf>) -> Result<PathBuf, AppError> {
    let raw = command.or(global).unwrap_or(std::env::current_dir()?);
    normalize_path(&raw)
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum ShellKind {
    Posix,
    PowerShell,
    Cmd,
}

fn detect_shell_kind() -> ShellKind {
    if cfg!(windows) {
        if std::env::var("PROMPT").is_ok() {
            return ShellKind::Cmd;
        }
        if std::env::var("POWERSHELL_DISTRIBUTION_CHANNEL").is_ok()
            || std::env::var("PSExecutionPolicyPreference").is_ok()
        {
            return ShellKind::PowerShell;
        }
        return ShellKind::PowerShell;
    }

    if let Ok(shell) = std::env::var("SHELL") {
        let shell = shell.to_ascii_lowercase();
        if shell.contains("pwsh") || shell.contains("powershell") {
            return ShellKind::PowerShell;
        }
    }
    ShellKind::Posix
}

fn escape_for_double_quotes(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

fn escape_for_powershell_double_quotes(value: &str) -> String {
    value
        .replace('`', "``")
        .replace('"', "`\"")
        .replace('$', "`$")
}

fn escape_for_cmd_set(value: &str) -> String {
    value.replace('"', "^\"")
}

fn normalize_scope_pattern(path: &Path) -> String {
    let mut pattern = path.to_string_lossy().to_string();
    if !contains_glob(&pattern) && !pattern.ends_with("/**") {
        if !pattern.ends_with('/') {
            pattern.push('/');
        }
        pattern.push_str("**");
    }
    pattern
}

fn contains_glob(value: &str) -> bool {
    value.contains('*') || value.contains('?') || value.contains('[')
}

fn resolve_version_spec(
    manifest: &Manifest,
    tool: &str,
    version: &str,
    target: &Target,
) -> Result<String, AppError> {
    manifest
        .resolve_version_spec(tool, version, target)
        .ok_or_else(|| AppError::Cache {
            message: format!(
                "no installer for {tool}@{version} ({}/{})",
                target.platform, target.arch
            ),
        })
}

fn normalize_tool_version_arg(tool: String, version: Option<String>) -> (String, Option<String>) {
    if version.is_some() {
        return (tool, version);
    }

    if let Some((name, ver)) = tool.rsplit_once('@') {
        if !name.is_empty() && !ver.is_empty() {
            return (name.to_string(), Some(ver.to_string()));
        }
    }

    (tool, None)
}

fn parse_tool_versions_file(path: &Path) -> Result<Vec<(String, String)>, AppError> {
    let contents = std::fs::read_to_string(path).map_err(|err| AppError::Config {
        message: format!("failed to read {}: {}", path.display(), err),
    })?;

    let mut result = Vec::new();
    for (line_num, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        let tool = parts.next().ok_or_else(|| AppError::Config {
            message: format!(
                "invalid format at line {} in {}",
                line_num + 1,
                path.display()
            ),
        })?;
        let version = parts.next().ok_or_else(|| AppError::Config {
            message: format!(
                "missing version at line {} in {}",
                line_num + 1,
                path.display()
            ),
        })?;

        result.push((tool.to_string(), version.to_string()));
    }

    Ok(result)
}

fn resolve_latest_version(
    manifest: &Manifest,
    tool: &str,
    target: &Target,
) -> Result<String, AppError> {
    let tool_entry = manifest.tools.iter().find(|entry| entry.name == tool);
    let Some(tool_entry) = tool_entry else {
        return Err(AppError::Cache {
            message: format!(
                "no installer for {tool}@latest ({}/{})",
                target.platform, target.arch
            ),
        });
    };

    let mut best: Option<(Version, String)> = None;
    for entry in tool_entry
        .versions
        .iter()
        .filter(|entry| entry.platform == target.platform && entry.arch == target.arch)
    {
        let parsed = match Version::parse(entry.ver.trim_start_matches('v')) {
            Ok(value) => value,
            Err(_) => continue,
        };

        match &best {
            Some((best_version, _)) if &parsed <= best_version => {}
            _ => best = Some((parsed, entry.ver.clone())),
        }
    }

    best.map(|(_, ver)| ver).ok_or_else(|| AppError::Cache {
        message: format!(
            "no installer for {tool}@latest ({}/{})",
            target.platform, target.arch
        ),
    })
}

fn upsert_scope_tool(config: &mut Config, pattern: &str, tool: &str, version: &str) {
    for scope in &mut config.scopes {
        if scope.pattern == pattern {
            scope.tools.insert(tool.to_string(), version.to_string());
            return;
        }
    }

    let mut tools = HashMap::new();
    tools.insert(tool.to_string(), version.to_string());
    config.scopes.push(Scope {
        pattern: pattern.to_string(),
        tools,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Global;

    fn map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn sample_manifest() -> Manifest {
        Manifest::parse(
            r#"
version = 1
generated_at = "2026-02-11T00:00:00Z"

[[tool]]
name = "node"
  [[tool.version]]
  ver = "22.1.0"
  platform = "macos"
  arch = "arm64"
  url = "https://example.com/node"
  sha256 = "deadbeef"
"#,
        )
        .expect("manifest")
    }

    #[test]
    fn path_and_pattern_helpers_work() {
        assert!(contains_glob("a/*"));
        assert!(!contains_glob("plain/path"));
        assert_eq!(
            normalize_scope_pattern(Path::new("/tmp/work")),
            "/tmp/work/**"
        );
        assert_eq!(
            normalize_scope_pattern(Path::new("/tmp/work/**")),
            "/tmp/work/**"
        );

        let resolved = resolve_path(None, Some(PathBuf::from("src"))).expect("resolve path");
        assert!(resolved.ends_with("src"));
    }

    #[test]
    fn resolve_version_spec_and_scope_upserts_work() {
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        let manifest = sample_manifest();
        let version = resolve_version_spec(&manifest, "node", "22", &target).expect("version");
        assert_eq!(version, "22.1.0");
        assert!(resolve_version_spec(&manifest, "node", "23", &target).is_err());

        let mut config = Config {
            global: Global {
                tools: map(&[("node", "20")]),
            },
            ..Default::default()
        };
        upsert_scope_tool(&mut config, "/tmp/work", "node", "22");
        upsert_scope_tool(&mut config, "/tmp/work", "bun", "1.0.0");
        assert_eq!(config.scopes.len(), 1);
        assert_eq!(
            config.scopes[0].tools.get("bun"),
            Some(&"1.0.0".to_string())
        );
    }

    #[test]
    fn parse_tool_versions_file_valid() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(
            &tool_versions_path,
            "node 20.10.0\npython 3.11.5\ngo 1.21.0\n",
        )
        .expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.11.5".to_string()));
        assert_eq!(result[2], ("go".to_string(), "1.21.0".to_string()));
    }

    #[test]
    fn parse_tool_versions_file_with_comments_and_empty_lines() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(
            &tool_versions_path,
            "# This is a comment\nnode 20.10.0\n\n# Another comment\npython 3.11.5\n\n",
        )
        .expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.11.5".to_string()));
    }

    #[test]
    fn parse_tool_versions_file_with_extra_whitespace() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(
            &tool_versions_path,
            "  node   20.10.0  \n  python   3.11.5  \n",
        )
        .expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("node".to_string(), "20.10.0".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.11.5".to_string()));
    }

    #[test]
    fn parse_tool_versions_file_missing_version() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(&tool_versions_path, "node\n").expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("missing version"));
        assert!(err_msg.contains("line 1"));
    }

    #[test]
    fn parse_tool_versions_file_not_found() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join("nonexistent.tool-versions");

        let result = parse_tool_versions_file(&tool_versions_path);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("failed to read"));
    }

    #[test]
    fn parse_tool_versions_file_empty() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(&tool_versions_path, "").expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn parse_tool_versions_file_only_comments() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let tool_versions_path = temp_dir.path().join(".tool-versions");

        std::fs::write(&tool_versions_path, "# comment 1\n# comment 2\n").expect("write file");

        let result = parse_tool_versions_file(&tool_versions_path).expect("parse");
        assert_eq!(result.len(), 0);
    }
}
