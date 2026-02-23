use std::env;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cache::Cache;
use crate::config::Config;
use crate::error::AppError;
use crate::manifest::{ManifestStore, Target};
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

pub fn run_doctor(
    config: &Config,
    cwd: &Path,
    config_path: &Path,
    cache_root: &Path,
    shims_root: &Path,
) -> Result<DoctorReport, AppError> {
    let shims_in_path = path_contains(&shims_root);
    let manifest = ManifestStore::new(cache_root, &config.manifest).load()?;
    let target = Target::current()?;
    let shim_names = list_shim_names(config, &manifest, &target);
    let conflicts = detect_conflicts(&shims_root, &shim_names);

    let cache = Cache::new(cache_root.to_path_buf());
    let resolve = resolve_tools(config, cwd)?;
    let mut missing_installs = Vec::new();
    for (tool, version) in resolve.tools {
        if !cache.is_installed(&tool, &version) {
            missing_installs.push(format!("{tool}@{version}"));
        }
    }
    missing_installs.sort();

    Ok(DoctorReport {
        config_path: config_path.to_path_buf(),
        cache_root: cache_root.to_path_buf(),
        shims_root: shims_root.to_path_buf(),
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
            let candidate = tool_in_dir(entry, tool);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_contains_detects_existing_path_entry() {
        let path_var = std::env::var("PATH").expect("PATH");
        let first = std::env::split_paths(&path_var)
            .next()
            .expect("first PATH entry");
        assert!(path_contains(&first));
    }

    #[test]
    fn detect_conflicts_and_tool_path_helpers_work() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("tools");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let tool_path = tool_in_dir(&dir, "node");
        std::fs::write(&tool_path, b"x").expect("write");

        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", dir.as_os_str());
        let conflicts = detect_conflicts(Path::new("/different"), &[String::from("node")]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0], tool_path);
        let no_conflicts = detect_conflicts(&dir, &[String::from("node")]);
        assert!(no_conflicts.is_empty());
        match original {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }
    }

    #[test]
    fn run_doctor_smoke_with_empty_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = Config::default();
        let cwd = std::env::current_dir().expect("cwd");
        let config_path = temp.path().join("config.toml");
        let cache_root = temp.path().join("cache");
        let shims_root = temp.path().join("shims");
        let report =
            run_doctor(&config, &cwd, &config_path, &cache_root, &shims_root).expect("doctor");
        assert_eq!(report.config_path, config_path);
        assert_eq!(report.cache_root, cache_root);
        assert_eq!(report.shims_root, shims_root);
    }
}
