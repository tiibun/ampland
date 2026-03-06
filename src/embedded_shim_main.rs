use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAIN_EXECUTABLE_PATH_FILE: &str = ".ampland-main-path";
const SHIM_TOOL_ENV_VAR: &str = "AMPLAND_SHIM_TOOL";

fn main() {
    if let Err(message) = run() {
        eprintln!("ampland shim error: {message}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let shim_path = env::current_exe().map_err(|err| err.to_string())?;
    let tool = tool_name(&shim_path)?;
    let ampland = resolve_main_executable(&shim_path)?;
    let status = Command::new(&ampland)
        .env(SHIM_TOOL_ENV_VAR, &tool)
        .args(env::args().skip(1))
        .status()
        .map_err(|err| format!("failed to launch {}: {err}", ampland.display()))?;

    std::process::exit(status.code().unwrap_or(1));
}

fn tool_name(shim_path: &Path) -> Result<String, String> {
    shim_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .ok_or_else(|| format!("could not determine tool name from {}", shim_path.display()))
}

fn resolve_main_executable(shim_path: &Path) -> Result<PathBuf, String> {
    let shims_root = shim_path
        .parent()
        .ok_or_else(|| format!("could not determine shims directory for {}", shim_path.display()))?;

    if let Some(configured) = configured_main_executable(shims_root)? {
        if configured.is_file() {
            return Ok(configured);
        }
    }

    find_main_executable_in_path(shim_path)?.ok_or_else(|| {
        format!(
            "could not find ampland executable; expected {} or an ampland binary in PATH",
            shims_root.join(MAIN_EXECUTABLE_PATH_FILE).display()
        )
    })
}

fn configured_main_executable(shims_root: &Path) -> Result<Option<PathBuf>, String> {
    let path = shims_root.join(MAIN_EXECUTABLE_PATH_FILE);
    match fs::read_to_string(&path) {
        Ok(contents) => {
            let value = contents.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(PathBuf::from(value)))
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("failed to read {}: {err}", path.display())),
    }
}

fn find_main_executable_in_path(shim_path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(path_var) = env::var_os("PATH") else {
        return Ok(None);
    };

    let shim_canonical = fs::canonicalize(shim_path).unwrap_or_else(|_| shim_path.to_path_buf());
    for entry in env::split_paths(&path_var) {
        let candidate = entry.join(ampland_executable_name());
        if !candidate.is_file() {
            continue;
        }
        let candidate_canonical = fs::canonicalize(&candidate).unwrap_or(candidate.clone());
        if candidate_canonical != shim_canonical {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

fn ampland_executable_name() -> &'static str {
    if cfg!(windows) {
        "ampland.exe"
    } else {
        "ampland"
    }
}
