use std::env;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cache::Cache;
use crate::config::Config;
use crate::error::AppError;
use crate::manifest::{ManifestStore, Target};
use crate::paths::{cache_dir, config_path, shims_dir};
use crate::resolve::resolve_tools;
use crate::shim::list_shim_names;

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub config_path: PathBuf,
    pub cache_root: PathBuf,
    pub shims_root: PathBuf,
    pub shims_in_path: bool,
    pub conflicts: Vec<PathBuf>,
    pub missing_installs: Vec<String>,
}

pub fn run_doctor(config: &Config, cwd: &Path) -> Result<DoctorReport, AppError> {
    let config_path = config_path()?;
    let cache_root = cache_dir()?;
    let shims_root = shims_dir()?;
    let shims_in_path = path_contains(&shims_root);
    let manifest = ManifestStore::new(&cache_root, &config.manifest).load()?;
    let target = Target::current()?;
    let shim_names = list_shim_names(config, &manifest, &target);
    let conflicts = detect_conflicts(&shims_root, &shim_names);

    let cache = Cache::new(cache_root.clone());
    let resolve = resolve_tools(config, cwd)?;
    let mut missing_installs = Vec::new();
    for (tool, version) in resolve.tools {
        if !cache.is_installed(&tool, &version) {
            missing_installs.push(format!("{tool}@{version}"));
        }
    }
    missing_installs.sort();

    Ok(DoctorReport {
        config_path,
        cache_root,
        shims_root,
        shims_in_path,
        conflicts,
        missing_installs,
    })
}

fn path_contains(target: &Path) -> bool {
    let path_var = env::var("PATH").unwrap_or_default();
    let entries: Vec<PathBuf> = env::split_paths(&path_var).collect();
    entries.iter().any(|entry| entry == target)
}

fn detect_conflicts(shims_root: &Path, shim_names: &[String]) -> Vec<PathBuf> {
    let mut conflicts = Vec::new();
    let path_var = env::var("PATH").unwrap_or_default();
    let entries: Vec<PathBuf> = env::split_paths(&path_var).collect();
    for tool in shim_names {
        for entry in &entries {
            let candidate = tool_in_dir(entry, &tool);
            if candidate.exists() {
                if entry != shims_root {
                    conflicts.push(candidate);
                }
                break;
            }
        }
    }
    conflicts
}

fn tool_in_dir(dir: &Path, tool: &str) -> PathBuf {
    let mut name = tool.to_string();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    dir.join(name)
}

