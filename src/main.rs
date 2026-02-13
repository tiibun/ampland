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
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::cache::Cache;
use crate::cli::{Cli, Command, ShimCommand};
use crate::config::{Config, LockFile, Scope};
use crate::doctor::run_doctor;
use crate::error::AppError;
use crate::installer::install;
use crate::manifest::{Manifest, ManifestStore, Target};
use crate::paths::{cache_dir, normalize_path, shims_dir};
use crate::resolve::{resolve_tool, resolve_tools, ResolutionSource};

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(err.exit_code());
    }
}

fn run() -> Result<(), AppError> {
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
    let _shims_root = cli.shims_dir.clone().unwrap_or(shims_dir()?);
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
            let version = resolve_version_spec(&manifest, &tool, &version, &target)?;
            let mut scope_label = None;
            if global {
                config
                    .global
                    .tools
                    .insert(tool.clone(), version.clone());
            } else {
                let cwd = resolve_path(cli.path, path)?;
                let pattern = normalize_scope_pattern(&cwd);
                upsert_scope_tool(&mut config, &pattern, &tool, &version);
                scope_label = Some(pattern);
            }
            if !cache.is_installed(&tool, &version) {
                let package = manifest
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
            config.save(&config_path)?;
            if !cli.quiet {
                if global {
                    println!("set {tool}@{version} for global");
                } else if let Some(pattern) = scope_label {
                    println!("set {tool}@{version} for {pattern}");
                }
            }
        }
        Command::Install { tool, version } => {
            let cwd = resolve_path(cli.path, None)?;
            let version = match version {
                Some(version) => version,
                None => resolve_tool(&config, &cwd, &tool)?.version,
            };
            let target = Target::current()?;
            let manifest = ManifestStore::new(&cache_root, &config.manifest).load()?;
            let version = resolve_version_spec(&manifest, &tool, &version, &target)?;
            let package = manifest
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
            cache.uninstall(&tool, &version)?;
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
                    .filter(|entry| {
                        entry.platform == target.platform && entry.arch == target.arch
                    })
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
                let display: Vec<String> = removed.iter().map(|path| path.display().to_string()).collect();
                println!("{}", serde_json::to_string_pretty(&display)?);
            } else if !cli.quiet {
                for path in removed {
                    println!("removed {}", path.display());
                }
            }
        }
        Command::Export {
            path,
            format,
            output,
        } => {
            let cwd = resolve_path(cli.path, path)?;
            let resolved = resolve_tools(&config, &cwd)?;
            let lock = LockFile::from_path_and_tools(&cwd, resolved.tools);
            let contents = lock.to_string(format)?;
            if let Some(out_path) = output {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(out_path, contents)?;
            } else if !cli.quiet {
                println!("{contents}");
            }
        }
        Command::Import { path, format, file } => {
            let contents = fs::read_to_string(&file)?;
            let mut lock = LockFile::parse(&contents, format)?;
            let scope_path = if let Some(path) = path {
                normalize_path(&path)?
            } else {
                normalize_path(Path::new(&lock.path))?
            };
            lock.path = normalize_scope_pattern(&scope_path);

            upsert_scope(&mut config, &lock)?;
            config.save(&config_path)?;
            if !cli.quiet {
                println!("imported scope {}", lock.path);
            }
        }
        Command::Doctor => {
            let cwd = resolve_path(cli.path, None)?;
            let report = run_doctor(&config, &cwd)?;
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
            let resolution = shim::resolve_bin_path(&config, &cwd, &tool, &cache, &manifest, &target)?;
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
        Command::UpdateManifest => {
            let store = ManifestStore::new(&cache_root, &config.manifest);
            let manifest = store.refresh()?;
            if cli.json {
                let mut output = BTreeMap::new();
                output.insert("version", manifest.version.to_string());
                output.insert("generated_at", manifest.generated_at);
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else if !cli.quiet {
                println!("manifest updated (version {})", manifest.version);
            }
        }
        Command::Shim { command } => match command {
            ShimCommand::Rebuild => {
                let created = shim::rebuild_shims(&config, cli.shims_dir.as_deref())?;
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
    }

    Ok(())
}

fn resolve_path(global: Option<PathBuf>, command: Option<PathBuf>) -> Result<PathBuf, AppError> {
    let raw = command.or(global).unwrap_or(std::env::current_dir()?);
    normalize_path(&raw)
}

fn normalize_scope_pattern(path: &Path) -> String {
    let mut pattern = path.to_string_lossy().to_string();
    if !contains_glob(&pattern) {
        if !pattern.ends_with("/**") {
            if !pattern.ends_with('/') {
                pattern.push('/');
            }
            pattern.push_str("**");
        }
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

fn upsert_scope(config: &mut Config, lock: &LockFile) -> Result<(), AppError> {
    let mut replaced = false;
    for scope in &mut config.scopes {
        if scope.pattern == lock.path {
            scope.tools = lock.tools.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        config.scopes.push(Scope {
            pattern: lock.path.clone(),
            tools: lock.tools.clone(),
        });
    }
    Ok(())
}

fn upsert_scope_tool(config: &mut Config, pattern: &str, tool: &str, version: &str) {
    for scope in &mut config.scopes {
        if scope.pattern == pattern {
            scope
                .tools
                .insert(tool.to_string(), version.to_string());
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
