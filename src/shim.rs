use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cache::Cache;
use crate::config::Config;
use crate::error::AppError;
use crate::paths::{cache_dir, shims_dir};
use crate::resolve::resolve_tool;

pub fn rebuild_shims(config: &Config, shims_override: Option<&Path>) -> Result<Vec<PathBuf>, AppError> {
    let shims_root = match shims_override {
        Some(path) => path.to_path_buf(),
        None => shims_dir()?,
    };
    fs::create_dir_all(&shims_root)?;

    let mut created = Vec::new();
    let exe = env::current_exe()?;
    for tool in all_tools(config) {
        let shim_path = shim_path_for(&shims_root, &tool);
        fs::copy(&exe, &shim_path)?;
        created.push(shim_path);
    }

    Ok(created)
}

pub fn add_shim(tool: &str, shims_override: Option<&Path>) -> Result<PathBuf, AppError> {
    let shims_root = match shims_override {
        Some(path) => path.to_path_buf(),
        None => shims_dir()?,
    };
    fs::create_dir_all(&shims_root)?;
    let exe = env::current_exe()?;
    let shim_path = shim_path_for(&shims_root, tool);
    fs::copy(&exe, &shim_path)?;
    Ok(shim_path)
}

pub fn run_as_shim(tool: &str) -> Result<(), AppError> {
    let (config, _) = Config::load(None)?;
    let cwd = env::current_dir()?;
    let resolution = resolve_tool(&config, &cwd, tool)?;
    let cache = Cache::new(cache_dir()?);
    let bin_path = cache.tool_bin_path(tool, &resolution.version);
    if !bin_path.exists() {
        return Err(AppError::ToolNotInstalled {
            tool: tool.to_string(),
        });
    }

    let args: Vec<String> = env::args().skip(1).collect();
    exec_tool(&bin_path, &args)
}

fn exec_tool(path: &Path, args: &[String]) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = Command::new(path).args(args).exec();
        Err(AppError::Other {
            message: format!("failed to exec {path:?}: {err}"),
        })
    }

    #[cfg(windows)]
    {
        let status = Command::new(path).args(args).status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn all_tools(config: &Config) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for tool in config.global.tools.keys() {
        set.insert(tool.clone());
    }
    for scope in &config.scopes {
        for tool in scope.tools.keys() {
            set.insert(tool.clone());
        }
    }
    set.into_iter().collect()
}

fn shim_path_for(root: &Path, tool: &str) -> PathBuf {
    let mut name = tool.to_string();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    root.join(name)
}
