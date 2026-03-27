use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use tempfile::NamedTempFile;

use crate::cache::Cache;
use crate::config::Config;
use crate::error::AppError;
use crate::manifest::{Manifest, ManifestStore, Target};
use crate::paths::{cache_dir, shims_dir};
use crate::resolve::resolve_tools;

const MANAGED_SHIMS_FILE: &str = ".ampland-managed-shims";
const MAIN_EXECUTABLE_PATH_FILE: &str = ".ampland-main-path";
const EMBEDDED_SHIM: &[u8] = include_bytes!(env!("AMPLAND_EMBEDDED_SHIM_PATH"));
pub const SHIM_TOOL_ENV_VAR: &str = "AMPLAND_SHIM_TOOL";

pub fn rebuild_shims(
    config: &Config,
    cache_root: &Path,
    shims_override: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    let shims_root = match shims_override {
        Some(path) => path.to_path_buf(),
        None => shims_dir()?,
    };
    fs::create_dir_all(&shims_root)?;
    let main_exe = current_main_executable()?;
    write_main_executable_path(&shims_root, &main_exe)?;

    let manifest = ManifestStore::new(cache_root, &config.manifest).load()?;
    let target = Target::current()?;
    let shim_names = list_shim_names(config, &manifest, &target);
    let mut expected_names: BTreeSet<String> = shim_names.iter().cloned().collect();
    let cache = Cache::new(cache_root.to_path_buf());
    for (tool, versions) in cache.list_installed()? {
        for version in versions {
            expected_names.extend(list_runtime_bin_names(&cache, &tool, &version)?);
        }
    }
    let mut managed = load_managed_shims(&shims_root)?;
    for stale in managed
        .iter()
        .filter(|name| !expected_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>()
    {
        let path = shim_path_for(&shims_root, &stale);
        if path.exists() {
            fs::remove_file(path)?;
        }
        managed.remove(&stale);
    }

    let mut created = Vec::new();
    for name in &expected_names {
        let shim_path = shim_path_for(&shims_root, name);
        write_embedded_shim(&shim_path)?;
        created.push(shim_path);
    }
    managed = expected_names;
    save_managed_shims(&shims_root, &managed)?;

    Ok(created)
}

pub fn add_shim(tool: &str, shims_override: Option<&Path>) -> Result<PathBuf, AppError> {
    let shims_root = match shims_override {
        Some(path) => path.to_path_buf(),
        None => shims_dir()?,
    };
    fs::create_dir_all(&shims_root)?;
    let main_exe = current_main_executable()?;
    write_main_executable_path(&shims_root, &main_exe)?;
    let shim_path = shim_path_for(&shims_root, tool);
    write_embedded_shim(&shim_path)?;
    Ok(shim_path)
}

pub fn run_as_shim(tool: &str) -> Result<(), AppError> {
    env::remove_var(SHIM_TOOL_ENV_VAR);
    let (config, _) = Config::load(None)?;
    let cwd = env::current_dir()?;
    let cache = Cache::new(cache_dir()?);
    let target = Target::current()?;
    let manifest = ManifestStore::new(cache.root(), &config.manifest).load()?;
    sync_runtime_shims(&config, &cwd, &cache, None)?;
    let resolution = resolve_bin_path(&config, &cwd, tool, &cache, &manifest, &target)?;

    let args: Vec<String> = env::args().skip(1).collect();
    let exit_code = exec_tool(&resolution.path, &args)?;
    if exit_code == 0 {
        sync_runtime_shims(&config, &cwd, &cache, None)?;
    }
    std::process::exit(exit_code);
}

fn exec_tool(path: &Path, args: &[String]) -> Result<i32, AppError> {
    let status = Command::new(path).args(args).status()?;
    Ok(exit_status_code(&status))
}

fn exit_status_code(status: &ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }

    1
}

fn sync_runtime_shims(
    config: &Config,
    cwd: &Path,
    cache: &Cache,
    shims_override: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    let shims_root = match shims_override {
        Some(path) => path.to_path_buf(),
        None => shims_dir()?,
    };
    fs::create_dir_all(&shims_root)?;
    let main_exe = current_main_executable()?;
    write_main_executable_path(&shims_root, &main_exe)?;
    let resolved = resolve_tools(config, cwd)?;
    let mut names = BTreeSet::new();
    for (tool, version) in resolved.tools {
        let runtime_names = list_runtime_bin_names(cache, &tool, &version)?;
        names.extend(runtime_names);
    }

    let mut managed = load_managed_shims(&shims_root)?;
    for stale in managed
        .iter()
        .filter(|name| !names.contains(*name))
        .cloned()
        .collect::<Vec<_>>()
    {
        let path = shim_path_for(&shims_root, &stale);
        if path.exists() {
            fs::remove_file(path)?;
        }
        managed.remove(&stale);
    }

    let mut created = Vec::new();
    for name in &names {
        let shim_path = shim_path_for(&shims_root, name);
        if shim_path.exists() && shim_matches_embedded_helper(&shim_path)? {
            continue;
        }
        write_embedded_shim(&shim_path)?;
        created.push(shim_path);
    }
    managed = names;
    save_managed_shims(&shims_root, &managed)?;
    Ok(created)
}

fn managed_shims_path(shims_root: &Path) -> PathBuf {
    shims_root.join(MANAGED_SHIMS_FILE)
}

fn main_executable_path_path(shims_root: &Path) -> PathBuf {
    shims_root.join(MAIN_EXECUTABLE_PATH_FILE)
}

fn load_managed_shims(shims_root: &Path) -> Result<BTreeSet<String>, AppError> {
    let path = managed_shims_path(shims_root);
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let contents = fs::read_to_string(path)?;
    let mut shims = BTreeSet::new();
    for line in contents.lines() {
        let value = line.trim();
        if !value.is_empty() {
            shims.insert(value.to_string());
        }
    }
    Ok(shims)
}

fn save_managed_shims(shims_root: &Path, shims: &BTreeSet<String>) -> Result<(), AppError> {
    let contents = if shims.is_empty() {
        String::new()
    } else {
        let mut value = shims.iter().cloned().collect::<Vec<_>>().join("\n");
        value.push('\n');
        value
    };
    fs::write(managed_shims_path(shims_root), contents)?;
    Ok(())
}

fn current_main_executable() -> Result<PathBuf, AppError> {
    let exe = env::current_exe()?;
    match fs::canonicalize(&exe) {
        Ok(path) => Ok(path),
        Err(_) => Ok(exe),
    }
}

fn write_main_executable_path(shims_root: &Path, path: &Path) -> Result<(), AppError> {
    let mut contents = path.to_string_lossy().into_owned();
    contents.push('\n');
    write_file_atomically(
        &main_executable_path_path(shims_root),
        contents.as_bytes(),
        false,
    )
}

fn write_embedded_shim(path: &Path) -> Result<(), AppError> {
    write_file_atomically(path, EMBEDDED_SHIM, true)
}

fn shim_matches_embedded_helper(path: &Path) -> Result<bool, AppError> {
    Ok(fs::read(path)? == EMBEDDED_SHIM)
}

fn write_file_atomically(path: &Path, contents: &[u8], executable: bool) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| AppError::Io {
        message: format!("path has no parent: {}", path.display()),
    })?;
    fs::create_dir_all(parent)?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(contents)?;
    set_file_permissions(temp.path(), executable)?;
    temp.persist(path)
        .map_err(|err| AppError::from(err.error))?;
    Ok(())
}

fn set_file_permissions(path: &Path, executable: bool) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = if executable { 0o755 } else { 0o644 };
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)?;
    }

    #[cfg(not(unix))]
    {
        let _ = (path, executable);
    }

    Ok(())
}

pub struct BinResolution {
    pub path: PathBuf,
}

pub fn resolve_bin_path(
    config: &Config,
    cwd: &Path,
    bin_name: &str,
    cache: &Cache,
    manifest: &Manifest,
    target: &Target,
) -> Result<BinResolution, AppError> {
    let resolved = resolve_tools(config, cwd)?;
    if let Some(version) = resolved.tools.get(bin_name) {
        return resolve_bin_for_tool(bin_name, version, bin_name, cache, manifest, target, false)
            .and_then(|resolution| {
                resolution.ok_or_else(|| {
                    let found_in = find_installed_versions_with_bin(bin_name, cache)
                        .unwrap_or_default();
                    let base = format!(
                        "bin '{bin_name}' not found for {bin_name}@{version} ({}/{})",
                        target.platform, target.arch
                    );
                    let message = if found_in.is_empty() {
                        base
                    } else {
                        format!(
                            "{base}\nFound in: {}\nHint: switch to a directory using that version, or run 'ampland use <version>' here.",
                            found_in.join(", ")
                        )
                    };
                    AppError::Cache { message }
                })
            });
    }

    let mut tools: Vec<(String, String)> = resolved.tools.into_iter().collect();
    tools.sort_by(|a, b| a.0.cmp(&b.0));
    for (tool, version) in tools {
        if let Some(resolution) =
            resolve_bin_for_tool(&tool, &version, bin_name, cache, manifest, target, true)?
        {
            return Ok(resolution);
        }
    }

    let found_in = find_installed_versions_with_bin(bin_name, cache)?;
    let message = if found_in.is_empty() {
        format!("no version configured for {bin_name}")
    } else {
        format!(
            "'{bin_name}' is not available in the current context.\nFound in: {}\nHint: switch to a directory using that version, or run 'ampland use <version>' here.",
            found_in.join(", ")
        )
    };
    Err(AppError::Config { message })
}

fn find_installed_versions_with_bin(
    bin_name: &str,
    cache: &Cache,
) -> Result<Vec<String>, AppError> {
    let mut found = Vec::new();
    for (tool, versions) in cache.list_installed()? {
        for version in versions {
            let version_dir = cache.tool_version_dir(&tool, &version);
            if find_runtime_bin(&version_dir, bin_name)?.is_some() {
                found.push(format!("{tool}@{version}"));
            }
        }
    }
    Ok(found)
}

fn resolve_bin_for_tool(
    tool: &str,
    version: &str,
    bin_name: &str,
    cache: &Cache,
    manifest: &Manifest,
    target: &Target,
    allow_missing_manifest: bool,
) -> Result<Option<BinResolution>, AppError> {
    let package = match manifest.resolve(tool, version, target) {
        Some(package) => package,
        None => {
            if allow_missing_manifest {
                return Ok(runtime_bin_path_for_name(cache, tool, version, bin_name)?
                    .map(|path| BinResolution { path }));
            }
            return Err(AppError::Cache {
                message: format!(
                    "no installer for {tool}@{version} ({}/{})",
                    target.platform, target.arch
                ),
            });
        }
    };

    let path = if package.bin_paths.is_empty() {
        if bin_name != tool {
            runtime_bin_path_for_name(cache, tool, version, bin_name)?
        } else {
            Some(cache.tool_bin_path(tool, version))
        }
    } else {
        match bin_path_for_name(cache, tool, version, bin_name, &package.bin_paths) {
            Some(path) => Some(path),
            None if allow_missing_manifest => {
                runtime_bin_path_for_name(cache, tool, version, bin_name)?
            }
            None => None,
        }
    };

    let path = match path {
        Some(path) => path,
        None => return Ok(None),
    };

    if path.exists() {
        return Ok(Some(BinResolution { path }));
    }

    if let Some(path) = runtime_bin_path_for_name(cache, tool, version, bin_name)? {
        return Ok(Some(BinResolution { path }));
    }

    Err(AppError::ToolNotInstalled {
        tool: tool.to_string(),
    })
}

fn runtime_bin_path_for_name(
    cache: &Cache,
    tool: &str,
    version: &str,
    bin_name: &str,
) -> Result<Option<PathBuf>, AppError> {
    let version_dir = cache.tool_version_dir(tool, version);
    if !version_dir.exists() {
        return Ok(None);
    }

    find_runtime_bin(&version_dir, bin_name)
}

fn bin_path_for_name(
    cache: &Cache,
    tool: &str,
    version: &str,
    bin_name: &str,
    bin_paths: &[String],
) -> Option<PathBuf> {
    let version_dir = cache.tool_version_dir(tool, version);
    bin_paths.iter().find_map(|path| {
        let stem = Path::new(path).file_stem()?.to_str()?;
        if stem != bin_name {
            return None;
        }
        Some(version_dir.join(path))
    })
}

fn list_runtime_bin_names(
    cache: &Cache,
    tool: &str,
    version: &str,
) -> Result<BTreeSet<String>, AppError> {
    let mut names = BTreeSet::new();
    let version_dir = cache.tool_version_dir(tool, version);
    if !version_dir.exists() {
        return Ok(names);
    }
    for dir in candidate_bin_dirs(&version_dir) {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(stem) = runtime_bin_stem(&path)? {
                names.insert(stem.to_string());
            }
        }
    }
    Ok(names)
}

fn find_runtime_bin(version_dir: &Path, bin_name: &str) -> Result<Option<PathBuf>, AppError> {
    for dir in candidate_bin_dirs(version_dir) {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if runtime_bin_stem(&path)?.as_deref() == Some(bin_name) {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn runtime_bin_stem(path: &Path) -> Result<Option<String>, AppError> {
    if !path.is_file() {
        return Ok(None);
    }
    if !is_runtime_executable(path)? {
        return Ok(None);
    }
    Ok(path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string()))
}

fn is_runtime_executable(path: &Path) -> Result<bool, AppError> {
    #[cfg(windows)]
    {
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        return Ok(matches!(
            ext.as_deref(),
            Some("exe" | "cmd" | "bat" | "com" | "ps1")
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode();
        Ok((mode & 0o111) != 0)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(true)
    }
}

fn candidate_bin_dirs(version_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![version_dir.to_path_buf(), version_dir.join("bin")];
    if cfg!(windows) {
        dirs.push(version_dir.join("Scripts"));
    }
    if let Ok(entries) = fs::read_dir(version_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            dirs.push(path.join("bin"));
            if cfg!(windows) {
                dirs.push(path.join("Scripts"));
            }
        }
    }
    dirs
}

pub fn list_shim_names(config: &Config, manifest: &Manifest, target: &Target) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    let tool_versions = config.all_tool_versions();
    for (tool, versions) in tool_versions {
        let mut added = false;
        for version in versions {
            if let Some(package) = manifest.resolve(&tool, &version, target) {
                let names = bin_names_from_paths(&package.bin_paths);
                if names.is_empty() {
                    set.insert(tool.clone());
                } else {
                    for name in names {
                        set.insert(name);
                    }
                }
                added = true;
            }
        }
        if !added {
            set.insert(tool);
        }
    }
    set.into_iter().collect()
}

fn bin_names_from_paths(bin_paths: &[String]) -> Vec<String> {
    let mut names = Vec::new();
    for path in bin_paths {
        if let Some(stem) = Path::new(path).file_stem().and_then(|value| value.to_str()) {
            names.push(stem.to_string());
        }
    }
    names
}

fn shim_path_for(root: &Path, tool: &str) -> PathBuf {
    let mut name = tool.to_string();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    root.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Global, Scope};

    fn mark_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(path, perms).expect("chmod");
        }
    }

    fn map(entries: &[(&str, &str)]) -> crate::config::ToolVersions {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn exit_status_code_maps_signal_to_shell_convention() {
        use std::os::unix::process::ExitStatusExt;

        let signaled = ExitStatus::from_raw(15);
        assert_eq!(exit_status_code(&signaled), 143);

        let exited = ExitStatus::from_raw(7 << 8);
        assert_eq!(exit_status_code(&exited), 7);
    }

    fn sample_manifest() -> Manifest {
        Manifest::parse(
            r#"
version = 1
generated_at = "2026-02-11T00:00:00Z"

[[tool]]
name = "node"
  [[tool.version]]
  ver = "22.0.0"
  platform = "macos"
  arch = "arm64"
  url = "https://example.com/node"
  sha256 = "deadbeef"
  bin_paths = ["bin/node", "bin/npm"]
"#,
        )
        .expect("manifest")
    }

    #[test]
    fn helper_functions_extract_expected_names_and_paths() {
        assert_eq!(
            bin_names_from_paths(&[String::from("a/node"), String::from("a/npm")]),
            vec!["node".to_string(), "npm".to_string()]
        );

        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        let bin = bin_path_for_name(
            &cache,
            "node",
            "22",
            "node",
            &[String::from("bin/node"), String::from("bin/npm")],
        )
        .expect("bin path");
        assert!(bin.ends_with("node/22/bin/node"));
        assert!(
            bin_path_for_name(&cache, "node", "22", "pnpm", &[String::from("bin/node")]).is_none()
        );

        let shim = shim_path_for(temp.path(), "node");
        assert!(shim.ends_with("node"));
    }

    #[test]
    fn list_shim_names_and_resolve_bin_path_work() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "22.0.0"), ("bun", "1.0.0")]),
            },
            scopes: vec![Scope {
                pattern: "*".to_string(),
                tools: map(&[("node", "22.0.0")]),
            }],
            ..Default::default()
        };
        let manifest = sample_manifest();
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };

        let names = list_shim_names(&config, &manifest, &target);
        assert!(names.contains(&"node".to_string()));
        assert!(names.contains(&"npm".to_string()));
        assert!(names.contains(&"bun".to_string()));

        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        let node_bin = cache
            .tool_version_dir("node", "22.0.0")
            .join("bin")
            .join("node");
        fs::create_dir_all(node_bin.parent().expect("parent")).expect("mkdir");
        fs::write(&node_bin, b"x").expect("write");

        let resolved = resolve_bin_path(
            &config,
            Path::new("workspace"),
            "node",
            &cache,
            &manifest,
            &target,
        )
        .expect("resolve bin");
        assert_eq!(resolved.path, node_bin);
    }

    #[test]
    fn add_shim_creates_embedded_file_when_override_is_used() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = add_shim("node", Some(temp.path())).expect("add shim");
        assert!(path.exists());
        assert_eq!(fs::read(&path).expect("read shim"), EMBEDDED_SHIM);
        assert_eq!(
            fs::read_to_string(main_executable_path_path(temp.path())).expect("read main path"),
            format!(
                "{}\n",
                current_main_executable()
                    .expect("current exe")
                    .to_string_lossy()
            )
        );
    }

    #[test]
    fn rebuild_shims_with_empty_config_returns_ok() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = Config::default();
        let created = rebuild_shims(&config, temp.path(), Some(temp.path())).expect("rebuild");
        assert!(created.is_empty());
    }

    #[test]
    fn rebuild_shims_prunes_only_managed_entries_with_empty_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let stale_path = shim_path_for(temp.path(), "node");
        let unmanaged_path = shim_path_for(temp.path(), "python");
        fs::write(&stale_path, b"shim").expect("write stale shim");
        fs::write(&unmanaged_path, b"user").expect("write unmanaged shim");
        let managed = BTreeSet::from([String::from("node")]);
        save_managed_shims(temp.path(), &managed).expect("save managed");
        let config = Config::default();

        let created = rebuild_shims(&config, temp.path(), Some(temp.path())).expect("rebuild");
        assert!(created.is_empty());
        assert!(!stale_path.exists());
        assert!(unmanaged_path.exists());
    }

    #[test]
    fn sync_runtime_shims_creates_missing_shims_from_bin_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().join("cache"));
        let shims = temp.path().join("shims");
        let config = Config {
            global: Global {
                tools: map(&[("node", "24.3.1")]),
            },
            ..Default::default()
        };
        let bin_dir = cache.tool_version_dir("node", "24.3.1").join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir");
        fs::write(bin_dir.join("node"), b"node").expect("write node");
        fs::write(bin_dir.join("pnpm"), b"pnpm").expect("write pnpm");
        mark_executable(&bin_dir.join("node"));
        mark_executable(&bin_dir.join("pnpm"));

        let created = sync_runtime_shims(&config, Path::new("workspace"), &cache, Some(&shims))
            .expect("sync shims");

        assert_eq!(created.len(), 2);
        assert!(shim_path_for(&shims, "node").exists());
        assert!(shim_path_for(&shims, "pnpm").exists());
        assert_eq!(
            fs::read(shim_path_for(&shims, "node")).expect("read node shim"),
            EMBEDDED_SHIM
        );
    }

    #[test]
    fn sync_runtime_shims_rewrites_existing_non_helper_shims() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().join("cache"));
        let shims = temp.path().join("shims");
        let config = Config {
            global: Global {
                tools: map(&[("node", "24.3.1")]),
            },
            ..Default::default()
        };
        let bin_dir = cache.tool_version_dir("node", "24.3.1").join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir");
        fs::write(bin_dir.join("node"), b"node").expect("write node");
        mark_executable(&bin_dir.join("node"));

        fs::create_dir_all(&shims).expect("mkdir shims");
        let old_shim = shim_path_for(&shims, "node");
        fs::write(&old_shim, b"old copied ampland").expect("write old shim");

        let created = sync_runtime_shims(&config, Path::new("workspace"), &cache, Some(&shims))
            .expect("sync shims");

        assert_eq!(created, vec![old_shim.clone()]);
        assert_eq!(fs::read(&old_shim).expect("read shim"), EMBEDDED_SHIM);
    }

    #[test]
    fn sync_runtime_shims_prunes_managed_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().join("cache"));
        let shims = temp.path().join("shims");
        let config = Config {
            global: Global {
                tools: map(&[("node", "24.3.1")]),
            },
            ..Default::default()
        };
        let bin_dir = cache.tool_version_dir("node", "24.3.1").join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir");
        fs::write(bin_dir.join("node"), b"node").expect("write node");
        fs::write(bin_dir.join("pnpm"), b"pnpm").expect("write pnpm");
        mark_executable(&bin_dir.join("node"));
        mark_executable(&bin_dir.join("pnpm"));
        sync_runtime_shims(&config, Path::new("workspace"), &cache, Some(&shims))
            .expect("initial sync");
        let unmanaged = shim_path_for(&shims, "python");
        fs::write(&unmanaged, b"external").expect("write unmanaged");

        fs::remove_file(bin_dir.join("pnpm")).expect("remove pnpm bin");
        sync_runtime_shims(&config, Path::new("workspace"), &cache, Some(&shims))
            .expect("second sync");

        assert!(shim_path_for(&shims, "node").exists());
        assert!(!shim_path_for(&shims, "pnpm").exists());
        assert!(unmanaged.exists());
    }

    #[cfg(unix)]
    #[test]
    fn sync_runtime_shims_ignores_non_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().join("cache"));
        let shims = temp.path().join("shims");
        let config = Config {
            global: Global {
                tools: map(&[("node", "24.3.1")]),
            },
            ..Default::default()
        };

        let version_dir = cache.tool_version_dir("node", "24.3.1");
        fs::create_dir_all(&version_dir).expect("mkdir");

        let node_bin = version_dir.join("node");
        fs::write(&node_bin, b"node").expect("write node");
        let mut node_perms = fs::metadata(&node_bin).expect("meta node").permissions();
        node_perms.set_mode(0o755);
        fs::set_permissions(&node_bin, node_perms).expect("chmod node");

        let readme = version_dir.join("README");
        fs::write(&readme, b"docs").expect("write readme");
        let mut readme_perms = fs::metadata(&readme).expect("meta readme").permissions();
        readme_perms.set_mode(0o644);
        fs::set_permissions(&readme, readme_perms).expect("chmod readme");

        sync_runtime_shims(&config, Path::new("workspace"), &cache, Some(&shims))
            .expect("sync shims");

        assert!(shim_path_for(&shims, "node").exists());
        assert!(!shim_path_for(&shims, "README").exists());
    }

    #[cfg(unix)]
    #[test]
    fn resolve_bin_path_falls_back_to_runtime_when_manifest_path_is_missing() {
        use std::os::unix::fs::PermissionsExt;

        let config = Config {
            global: Global {
                tools: map(&[("node", "22.22.0")]),
            },
            ..Default::default()
        };
        let manifest = Manifest::parse(
            r#"
version = 1
generated_at = "2026-02-11T00:00:00Z"

[[tool]]
name = "node"
  [[tool.version]]
  ver = "22.22.0"
  platform = "windows"
  arch = "x64"
  url = "https://example.com/node"
  sha256 = "deadbeef"
  bin_paths = ["node-v22.22.0-win-x64/node.exe"]
"#,
        )
        .expect("manifest");
        let target = Target {
            platform: "windows".to_string(),
            arch: "x64".to_string(),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());

        let runtime_node = cache.tool_version_dir("node", "22.22.0").join("node.exe");
        fs::create_dir_all(runtime_node.parent().expect("parent")).expect("mkdir");
        fs::write(&runtime_node, b"node").expect("write");
        let mut perms = fs::metadata(&runtime_node).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&runtime_node, perms).expect("chmod");

        let resolved = resolve_bin_path(
            &config,
            Path::new("workspace"),
            "node",
            &cache,
            &manifest,
            &target,
        )
        .expect("resolve runtime fallback");

        assert_eq!(resolved.path, runtime_node);
    }

    #[test]
    fn resolve_bin_path_returns_cache_error_when_direct_bin_missing_in_package() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "22.0.0")]),
            },
            ..Default::default()
        };
        let manifest = Manifest::parse(
            r#"
version = 1
generated_at = "2026-02-11T00:00:00Z"

[[tool]]
name = "node"
  [[tool.version]]
  ver = "22.0.0"
  platform = "macos"
  arch = "arm64"
  url = "https://example.com/node"
  sha256 = "deadbeef"
  bin_paths = ["bin/npm"]
"#,
        )
        .expect("manifest");
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        let result = resolve_bin_path(
            &config,
            Path::new("workspace"),
            "node",
            &cache,
            &manifest,
            &target,
        );
        let err = match result {
            Ok(_) => panic!("expected error"),
            Err(err) => err,
        };
        assert!(matches!(err, AppError::Cache { .. }));
    }

    #[test]
    fn resolve_bin_path_finds_runtime_bin_not_listed_in_manifest() {
        let config = Config {
            global: Global {
                tools: map(&[("node", "22.0.0")]),
            },
            ..Default::default()
        };
        let manifest = sample_manifest();
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        let tsc_bin = cache
            .tool_version_dir("node", "22.0.0")
            .join("bin")
            .join("tsc");
        fs::create_dir_all(tsc_bin.parent().expect("parent")).expect("mkdir");
        fs::write(&tsc_bin, b"x").expect("write");
        mark_executable(&tsc_bin);

        let resolved = resolve_bin_path(
            &config,
            Path::new("workspace"),
            "tsc",
            &cache,
            &manifest,
            &target,
        )
        .expect("resolve runtime bin");
        assert_eq!(resolved.path, tsc_bin);
    }

    #[test]
    fn find_installed_versions_with_bin_returns_matching_versions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());

        // node@24 has eslint installed
        let eslint = cache
            .tool_version_dir("node", "24.0.0")
            .join("bin")
            .join("eslint");
        fs::create_dir_all(eslint.parent().expect("parent")).expect("mkdir");
        fs::write(&eslint, b"x").expect("write");
        mark_executable(&eslint);
        fs::write(
            cache.tool_version_dir("node", "24.0.0").join(".installed"),
            b"",
        )
        .expect("marker");

        // node@22 does NOT have eslint
        let node22_dir = cache.tool_version_dir("node", "22.0.0");
        fs::create_dir_all(&node22_dir).expect("mkdir");
        fs::write(node22_dir.join(".installed"), b"").expect("marker");

        let found =
            find_installed_versions_with_bin("eslint", &cache).expect("find_installed");
        assert_eq!(found, vec!["node@24.0.0".to_string()]);
    }

    #[test]
    fn resolve_bin_path_suggests_other_version_when_bin_not_in_current_context() {
        // Current context: node@22 (no eslint)
        // Installed: node@24 has eslint
        let config = Config {
            global: Global {
                tools: map(&[("node", "22.0.0")]),
            },
            ..Default::default()
        };
        let manifest = sample_manifest();
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());

        // Install node@24 with eslint
        let eslint = cache
            .tool_version_dir("node", "24.0.0")
            .join("bin")
            .join("eslint");
        fs::create_dir_all(eslint.parent().expect("parent")).expect("mkdir");
        fs::write(&eslint, b"x").expect("write");
        mark_executable(&eslint);
        fs::write(
            cache.tool_version_dir("node", "24.0.0").join(".installed"),
            b"",
        )
        .expect("marker");

        let result = resolve_bin_path(
            &config,
            Path::new("workspace"),
            "eslint",
            &cache,
            &manifest,
            &target,
        );

        let msg = match result {
            Err(AppError::Config { message }) => message,
            Ok(_) => panic!("expected error"),
            Err(other) => panic!("unexpected error: {other:?}"),
        };
        assert!(msg.contains("node@24.0.0"), "hint missing: {msg}");
        assert!(msg.contains("Hint:"), "hint line missing: {msg}");
    }
}
