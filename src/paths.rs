use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::error::AppError;

pub fn config_path() -> Result<PathBuf, AppError> {
    let base = BaseDirs::new().ok_or_else(|| AppError::Config {
        message: "unable to determine home directory".to_string(),
    })?;
    let config_dir = base.config_local_dir();
    Ok(config_dir.join("ampland").join("config.toml"))
}

pub fn cache_dir() -> Result<PathBuf, AppError> {
    let base = BaseDirs::new().ok_or_else(|| AppError::Cache {
        message: "unable to determine home directory".to_string(),
    })?;
    if cfg!(windows) {
        let local = base.data_local_dir();
        Ok(local.join("ampland").join("cache"))
    } else {
        let home = base.home_dir();
        Ok(home.join(".local").join("ampland").join("cache"))
    }
}

pub fn shims_dir() -> Result<PathBuf, AppError> {
    let base = BaseDirs::new().ok_or_else(|| AppError::Cache {
        message: "unable to determine home directory".to_string(),
    })?;
    if cfg!(windows) {
        let local = base.data_local_dir();
        Ok(local.join("ampland").join("shims"))
    } else {
        let home = base.home_dir();
        Ok(home.join(".local").join("ampland").join("shims"))
    }
}

pub fn expand_tilde(path: &str) -> Result<String, AppError> {
    if !path.starts_with("~") {
        return Ok(path.to_string());
    }
    let base = BaseDirs::new().ok_or_else(|| AppError::Config {
        message: "unable to determine home directory".to_string(),
    })?;
    let home = base.home_dir().to_string_lossy();
    if path == "~" {
        Ok(home.to_string())
    } else if let Some(rest) = path.strip_prefix("~/") {
        Ok(format!("{home}/{rest}"))
    } else {
        Ok(path.to_string())
    }
}

pub fn normalize_path(path: &Path) -> Result<PathBuf, AppError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(absolute)
}
