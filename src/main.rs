mod cache;
mod cli;
mod config;
mod doctor;
mod error;
mod paths;
mod resolve;
mod shim;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::cache::Cache;
use crate::cli::{Cli, Command, ShimCommand};
use crate::config::{Config, LockFile, Scope};
use crate::doctor::run_doctor;
use crate::error::AppError;
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
    if exec_name != "ampland" {
        return shim::run_as_shim(exec_name);
    }

    let cli = Cli::parse();
    let (mut config, config_path) = Config::load(cli.config.as_deref())?;
    let cache_root = cli.cache_dir.unwrap_or(cache_dir()?);
    let _shims_root = cli.shims_dir.clone().unwrap_or(shims_dir()?);
    let cache = Cache::new(cache_root.clone());

    match cli.command {
        Command::Install { tool, version } => {
            let cwd = resolve_path(cli.path, None)?;
            let version = match version {
                Some(version) => version,
                None => resolve_tool(&config, &cwd, &tool)?.version,
            };
            let bin_path = cache.install_placeholder(&tool, &version)?;
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
            let resolution = resolve_tool(&config, &cwd, &tool)?;
            let bin_path = cache.tool_bin_path(&tool, &resolution.version);
            if !bin_path.exists() {
                return Err(AppError::ToolNotInstalled { tool });
            }
            if !cli.quiet {
                println!("{}", bin_path.display());
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
