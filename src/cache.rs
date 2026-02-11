use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

use fs2::FileExt;

use crate::error::AppError;

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Cache { root }
    }

    pub fn tool_version_dir(&self, tool: &str, version: &str) -> PathBuf {
        self.root.join(tool).join(version)
    }

    pub fn tool_bin_path(&self, tool: &str, version: &str) -> PathBuf {
        let mut name = tool.to_string();
        if cfg!(windows) {
            name.push_str(".exe");
        }
        self.tool_version_dir(tool, version).join("bin").join(name)
    }

    pub fn is_installed(&self, tool: &str, version: &str) -> bool {
        self.tool_bin_path(tool, version).exists()
    }

    pub fn install_placeholder(&self, tool: &str, version: &str) -> Result<PathBuf, AppError> {
        let lock_path = self.root.join(".lock");
        fs::create_dir_all(&self.root)?;
        let lock_file = File::create(&lock_path)?;
        lock_file.lock_exclusive()?;

        let bin_path = self.tool_bin_path(tool, version);
        if let Some(parent) = bin_path.parent() {
            fs::create_dir_all(parent)?;
        }

        if !bin_path.exists() {
            let mut file = File::create(&bin_path)?;
            writeln!(
                file,
                "ampland placeholder for {tool} {version}. Replace with a real installer."
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = file.metadata()?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&bin_path, perms)?;
            }
        }

        fs2::FileExt::unlock(&lock_file)?;
        Ok(bin_path)
    }

    pub fn uninstall(&self, tool: &str, version: &str) -> Result<(), AppError> {
        let dir = self.tool_version_dir(tool, version);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
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
                if version.path().is_dir() {
                    versions.push(version.file_name().to_string_lossy().to_string());
                }
            }
            versions.sort();
            result.push((tool, versions));
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
