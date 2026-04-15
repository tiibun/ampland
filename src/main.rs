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
mod tool_version_file;
mod updater;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Parser;
use semver::Version;

use crate::cache::Cache;
use crate::cli::{ActivateShell, Cli, Command, ConfigCommand, ShimCommand};
use crate::config::{Config, Scope, ToolVersions};
use crate::doctor::run_doctor;
use crate::error::AppError;
use crate::installer::install;
use crate::manifest::{load_manifest, Manifest, Target};
use crate::paths::{cache_dir, is_path_spec, normalize_path, shims_dir};
use crate::resolve::{resolve_tool, ResolutionSource};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(err.exit_code());
    }
}

fn run() -> Result<(), AppError> {
    #[cfg(windows)]
    updater::cleanup_old_binary();

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
            let manifest = load_manifest()?;
            let cwd = resolve_path(cli.path.clone(), path.clone())?;

            // If tool is None, read from .tool-versions
            let tools_to_set = if let Some(tool) = tool {
                let (tool, version) = normalize_tool_version_arg(tool, version);
                let version = match version {
                    Some(v) => v,
                    None => resolve_latest_version(&manifest, &tool, &target)?,
                };
                vec![(tool, version)]
            } else {
                let tool_versions_path = cwd.join(".tool-versions");
                let mise_toml_path = cwd.join("mise.toml");
                let package_json_path = cwd.join("package.json");

                if tool_versions_path.exists() {
                    tool_version_file::parse_tool_versions_file(&tool_versions_path)?
                } else if mise_toml_path.exists() {
                    tool_version_file::parse_mise_toml_file(&mise_toml_path)?
                } else if package_json_path.exists() {
                    tool_version_file::parse_volta_from_package_json(&package_json_path)?
                } else {
                    return Err(AppError::Config {
                        message: format!(
                            "no tool version file found at {} (checked .tool-versions, mise.toml, package.json)",
                            cwd.display()
                        ),
                    });
                }
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
                    if is_path_spec(&version) {
                        let path = std::path::Path::new(&version);
                        if !path.exists() {
                            return Err(AppError::Config {
                                message: format!(
                                    "specified path for {tool} does not exist: {version}"
                                ),
                            });
                        }
                    } else {
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
        Command::Unuse { tool, global, path } => {
            let cwd = resolve_path(cli.path.clone(), path)?;

            let location_label;
            if global {
                if config.global.tools.remove(&tool).is_none() {
                    return Err(AppError::Config {
                        message: format!("{tool} is not set in global config"),
                    });
                }
                location_label = "global".to_string();
            } else {
                let pattern = normalize_scope_pattern(&cwd);
                if !config.remove_tool_from_scope(&pattern, &tool)? {
                    return Err(AppError::Config {
                        message: format!("{tool} is not set for {pattern}"),
                    });
                }
                location_label = pattern;
            }

            config.save(&config_path)?;

            if !config.all_tool_versions().contains_key(&tool) {
                shim::rebuild_shims(&config, &cache_root, cli.shims_dir.as_deref())?;
            }

            if !cli.quiet {
                println!("removed {tool} from {location_label}");
            }
        }
        Command::Install { tool, version } => {
            let (tool, version) = normalize_tool_version_arg(tool, version);
            let target = Target::current()?;
            let manifest = load_manifest()?;
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
            let cwd = resolve_path(cli.path.clone(), None)?;
            let version = match version {
                Some(v) => v,
                None => {
                    resolve_tool(&config, &cwd, &tool)
                        .map(|r| r.version)
                        .map_err(|_| AppError::Config {
                            message: format!(
                                "no active version found for {tool}; specify a version (e.g. ampland uninstall {tool} 22)"
                            ),
                        })?
                }
            };
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
            let manifest = load_manifest()?;
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
                println!("shims early in PATH: {}", report.shims_early_in_path);
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
            let manifest = load_manifest()?;
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
        Command::Activate { shell } => {
            if !cli.quiet {
                let shell = ShellKind::from(shell);
                let shims_value = shims_root.to_string_lossy();
                match shell {
                    ShellKind::Posix => {
                        let value = escape_for_double_quotes(&shims_value);
                        println!("export PATH=\"{}:$PATH\"", value);
                    }
                    ShellKind::Fish => {
                        let value = escape_for_double_quotes(&shims_value);
                        println!("set -gx PATH \"{}\" $PATH", value);
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
        Command::Update { version, yes } => {
            updater::self_update(version.as_deref(), yes, cli.quiet)?;
        }
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
    Fish,
    PowerShell,
    Cmd,
}

impl From<ActivateShell> for ShellKind {
    fn from(value: ActivateShell) -> Self {
        match value {
            ActivateShell::Posix => Self::Posix,
            ActivateShell::Fish => Self::Fish,
            ActivateShell::PowerShell => Self::PowerShell,
            ActivateShell::Cmd => Self::Cmd,
        }
    }
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
    if is_path_spec(version) {
        return Ok(version.to_string());
    }
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

    let mut tools = ToolVersions::new();
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

    fn map(entries: &[(&str, &str)]) -> ToolVersions {
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
    fn is_path_spec_detects_absolute_paths() {
        assert!(is_path_spec("/usr/local/bin/node"));
        assert!(is_path_spec("/home/user/.local/bin/node"));
        #[cfg(windows)]
        assert!(is_path_spec(r"C:\Program Files\nodejs\node.exe"));
        #[cfg(windows)]
        assert!(is_path_spec("C:/Program Files/nodejs/node.exe"));
        assert!(!is_path_spec("22.0.0"));
        assert!(!is_path_spec("22"));
        assert!(!is_path_spec("latest"));
    }

    #[test]
    fn resolve_version_spec_returns_absolute_path_as_is() {
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        let manifest = sample_manifest();
        let version =
            resolve_version_spec(&manifest, "node", "/usr/local/bin/node", &target).expect("ok");
        assert_eq!(version, "/usr/local/bin/node");
    }
}
