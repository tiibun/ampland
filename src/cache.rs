use std::fs::{self, File};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::AppError;

pub(crate) const INSTALL_MARKER_FILE: &str = ".installed";

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Cache { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn tool_version_dir(&self, tool: &str, version: &str) -> PathBuf {
        self.root.join(tool).join(version)
    }

    pub fn tool_bin_path(&self, tool: &str, version: &str) -> PathBuf {
        let mut name = tool.to_string();
        if cfg!(windows) {
            name.push_str(".exe");
        }
        self.tool_version_dir(tool, version).join(name)
    }

    pub fn is_installed(&self, tool: &str, version: &str) -> bool {
        let dir = self.tool_version_dir(tool, version);
        if !dir.is_dir() {
            return false;
        }
        dir.join(INSTALL_MARKER_FILE).is_file()
    }

    pub fn with_lock<T, F: FnOnce() -> Result<T, AppError>>(&self, f: F) -> Result<T, AppError> {
        let lock_path = self.root.join(".lock");
        fs::create_dir_all(&self.root)?;
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive()?;

        let result = f();
        let _ = lock_file.unlock();
        result
    }

    pub fn uninstall(&self, tool: &str, version: &str) -> Result<(), AppError> {
        let dir = self.tool_version_dir(tool, version);
        if !dir.exists() {
            return Err(AppError::Cache {
                message: format!("{tool}@{version} is not installed"),
            });
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    pub fn list_installed(&self) -> Result<Vec<(String, Vec<String>)>, AppError> {
        let mut result = Vec::new();
        if !self.root.exists() {
            return Ok(result);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let tool = entry.file_name().to_string_lossy().to_string();
            let mut versions = Vec::new();
            for version in fs::read_dir(&path)? {
                let version = version?;
                let version_path = version.path();
                if version_path.is_dir() && version_path.join(INSTALL_MARKER_FILE).is_file() {
                    versions.push(version.file_name().to_string_lossy().to_string());
                }
            }
            versions.sort();
            if !versions.is_empty() {
                result.push((tool, versions));
            }
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(result)
    }

    pub fn gc(
        &self,
        keep: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    ) -> Result<Vec<PathBuf>, AppError> {
        let mut removed = Vec::new();
        if !self.root.exists() {
            return Ok(removed);
        }
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let tool_path = entry.path();
            if !tool_path.is_dir() {
                continue;
            }
            let tool = entry.file_name().to_string_lossy().to_string();
            let keep_versions = keep.get(&tool);
            for version_entry in fs::read_dir(&tool_path)? {
                let version_entry = version_entry?;
                let version_path = version_entry.path();
                if !version_path.is_dir() {
                    continue;
                }
                let version = version_entry.file_name().to_string_lossy().to_string();
                let keep_version = keep_versions
                    .map(|set| set.contains(&version))
                    .unwrap_or(false);
                if !keep_version {
                    fs::remove_dir_all(&version_path)?;
                    removed.push(version_path);
                }
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn path_helpers_and_install_state_work() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        assert_eq!(cache.root(), temp.path());
        assert!(cache.tool_version_dir("node", "22").ends_with("node/22"));

        assert!(!cache.is_installed("node", "22"));
        let bin = cache.tool_bin_path("node", "22");
        fs::create_dir_all(bin.parent().expect("parent")).expect("mkdir");
        fs::write(&bin, b"bin").expect("write");
        assert!(!cache.is_installed("node", "22"));
        fs::write(
            cache
                .tool_version_dir("node", "22")
                .join(INSTALL_MARKER_FILE),
            b"ok",
        )
        .expect("write marker");
        assert!(cache.is_installed("node", "22"));
    }

    #[test]
    fn with_lock_uninstall_list_and_gc_work() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());

        let value = cache
            .with_lock(|| Ok::<_, AppError>(123))
            .expect("with_lock");
        assert_eq!(value, 123);

        for (tool, version) in [("node", "20"), ("node", "22"), ("bun", "1")] {
            let bin = cache.tool_bin_path(tool, version);
            fs::create_dir_all(bin.parent().expect("parent")).expect("mkdir");
            fs::write(bin, b"x").expect("write");
            fs::write(
                cache
                    .tool_version_dir(tool, version)
                    .join(INSTALL_MARKER_FILE),
                b"ok",
            )
            .expect("write marker");
        }
        let listed = cache.list_installed().expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "bun");
        assert_eq!(listed[1].0, "node");

        cache.uninstall("bun", "1").expect("uninstall");
        assert!(!cache.tool_version_dir("bun", "1").exists());

        let mut keep = HashMap::<String, HashSet<String>>::new();
        keep.insert("node".into(), HashSet::from(["22".to_string()]));
        let removed = cache.gc(&keep).expect("gc");
        assert_eq!(removed.len(), 1);
        assert!(cache.tool_version_dir("node", "22").exists());
        assert!(!cache.tool_version_dir("node", "20").exists());
    }

    #[test]
    fn uninstall_missing_version_returns_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        let err = cache
            .uninstall("node", "99")
            .expect_err("missing install should fail");
        assert!(matches!(err, AppError::Cache { .. }));
    }

    #[test]
    fn list_installed_ignores_incomplete_versions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());

        let complete = cache.tool_bin_path("node", "22");
        fs::create_dir_all(complete.parent().expect("parent")).expect("mkdir");
        fs::write(&complete, b"x").expect("write node");
        fs::write(
            cache
                .tool_version_dir("node", "22")
                .join(INSTALL_MARKER_FILE),
            b"ok",
        )
        .expect("write marker");

        let partial = cache.tool_bin_path("node", "23");
        fs::create_dir_all(partial.parent().expect("parent")).expect("mkdir");
        fs::write(&partial, b"x").expect("write partial");

        let listed = cache.list_installed().expect("list");
        assert_eq!(listed, vec![("node".to_string(), vec!["22".to_string()])]);
    }
}
