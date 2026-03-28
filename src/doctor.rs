use std::env;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cache::Cache;
use crate::config::Config;
use crate::error::AppError;
use crate::manifest::{load_manifest, Target};
use crate::resolve::resolve_tools;
use crate::shim::list_shim_names;

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub config_path: PathBuf,
    pub cache_root: PathBuf,
    pub shims_root: PathBuf,
    pub shims_in_path: bool,
    pub shims_early_in_path: bool,
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
    let path_entries = path_entries();
    let shims_in_path = path_contains(shims_root, &path_entries);
    let manifest = load_manifest()?;
    let target = Target::current()?;
    let shim_names = list_shim_names(config, &manifest, &target);
    let conflicts = detect_conflicts(shims_root, &shim_names, &path_entries);
    let shims_early_in_path = shims_in_path && conflicts.is_empty();

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
        shims_early_in_path,
        conflicts,
        missing_installs,
    })
}

fn path_entries() -> Vec<PathBuf> {
    let path_var = env::var("PATH").unwrap_or_default();
    env::split_paths(&path_var).collect()
}

fn path_contains(target: &Path, entries: &[PathBuf]) -> bool {
    entries.iter().any(|entry| entry == target)
}

fn detect_conflicts(shims_root: &Path, shim_names: &[String], entries: &[PathBuf]) -> Vec<PathBuf> {
    let mut conflicts = Vec::new();
    let shims_index = entries.iter().position(|entry| entry == shims_root);
    for tool in shim_names {
        for (index, entry) in entries.iter().enumerate() {
            if Some(index) == shims_index {
                break;
            }
            let candidate = tool_in_dir(entry, tool);
            if candidate.exists() {
                conflicts.push(candidate);
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
        let entries: Vec<_> = std::env::split_paths(&path_var).collect();
        let first = entries.first().expect("first PATH entry");
        assert!(path_contains(first, &entries));
    }

    #[test]
    fn detect_conflicts_and_tool_path_helpers_work() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("tools");
        let shims = temp.path().join("shims");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::create_dir_all(&shims).expect("mkdir shims");
        let tool_path = tool_in_dir(&dir, "node");
        std::fs::write(&tool_path, b"x").expect("write");

        let entries = vec![dir.clone(), shims.clone()];
        let conflicts = detect_conflicts(&shims, &[String::from("node")], &entries);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0], tool_path);

        let entries = vec![shims.clone(), dir];
        let no_conflicts = detect_conflicts(&shims, &[String::from("node")], &entries);
        assert!(no_conflicts.is_empty());
    }

    #[test]
    fn run_doctor_reports_when_shims_are_not_early() {
        use crate::config::Global;

        let temp = tempfile::tempdir().expect("tempdir");
        let tool_dir = temp.path().join("tools");
        let shims_root = temp.path().join("shims");
        std::fs::create_dir_all(&tool_dir).expect("mkdir tools");
        std::fs::create_dir_all(&shims_root).expect("mkdir shims");
        std::fs::write(tool_in_dir(&tool_dir, "node"), b"x").expect("write node");

        let original = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            std::env::join_paths([tool_dir.as_path(), shims_root.as_path()]).expect("join PATH"),
        );

        let config = Config {
            global: Global {
                tools: [("node".to_string(), "25.8.0".to_string())]
                    .into_iter()
                    .collect(),
            },
            ..Default::default()
        };
        let cwd = std::env::current_dir().expect("cwd");
        let config_path = temp.path().join("config.toml");
        let cache_root = temp.path().join("cache");
        let report =
            run_doctor(&config, &cwd, &config_path, &cache_root, &shims_root).expect("doctor");
        assert!(report.shims_in_path);
        assert!(!report.shims_early_in_path);

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
        assert!(!report.shims_early_in_path);
    }
}
