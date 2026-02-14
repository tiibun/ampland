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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_expected_standard_paths() {
        assert!(config_path()
            .expect("config path")
            .ends_with("ampland/config.toml"));
        assert!(cache_dir().expect("cache path").ends_with("ampland/cache"));
        assert!(shims_dir().expect("shims path").ends_with("ampland/shims"));
    }

    #[test]
    fn expands_tilde_variants() {
        let base = directories::BaseDirs::new().expect("base dirs");
        let home = base.home_dir().to_string_lossy().to_string();
        assert_eq!(expand_tilde("plain").expect("no tilde"), "plain");
        assert_eq!(expand_tilde("~").expect("home"), home);
        assert_eq!(
            expand_tilde("~/work").expect("home child"),
            format!("{home}/work")
        );
        assert_eq!(expand_tilde("~user").expect("unknown user form"), "~user");
    }

    #[test]
    fn normalizes_relative_and_absolute_paths() {
        let absolute = Path::new("/tmp");
        assert_eq!(normalize_path(absolute).expect("absolute"), absolute);

        let rel = Path::new("src");
        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(normalize_path(rel).expect("relative"), cwd.join(rel));
    }
}
