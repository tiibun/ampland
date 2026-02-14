use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cache::Cache;
use crate::config::Config;
use crate::error::AppError;
use crate::manifest::{Manifest, ManifestStore, Target};
use crate::paths::{cache_dir, shims_dir};
use crate::resolve::resolve_tools;

pub fn rebuild_shims(
    config: &Config,
    shims_override: Option<&Path>,
) -> Result<Vec<PathBuf>, AppError> {
    let shims_root = match shims_override {
        Some(path) => path.to_path_buf(),
        None => shims_dir()?,
    };
    fs::create_dir_all(&shims_root)?;

    let cache_root = cache_dir()?;
    let manifest = ManifestStore::new(&cache_root, &config.manifest).load()?;
    let target = Target::current()?;
    let shim_names = list_shim_names(config, &manifest, &target);

    let mut created = Vec::new();
    let exe = env::current_exe()?;
    for name in shim_names {
        let shim_path = shim_path_for(&shims_root, &name);
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
    let cache = Cache::new(cache_dir()?);
    let target = Target::current()?;
    let manifest = ManifestStore::new(cache.root(), &config.manifest).load()?;
    let resolution = resolve_bin_path(&config, &cwd, tool, &cache, &manifest, &target)?;

    let args: Vec<String> = env::args().skip(1).collect();
    exec_tool(&resolution.path, &args)
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
                resolution.ok_or_else(|| AppError::Cache {
                    message: format!(
                        "bin '{bin_name}' not found for {bin_name}@{version} ({}/{})",
                        target.platform, target.arch
                    ),
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

    Err(AppError::Config {
        message: format!("no version configured for {bin_name}"),
    })
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
                return Ok(None);
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
            return Ok(None);
        }
        Some(cache.tool_bin_path(tool, version))
    } else {
        bin_path_for_name(cache, tool, version, bin_name, &package.bin_paths)
    };

    let path = match path {
        Some(path) => path,
        None => return Ok(None),
    };

    if !path.exists() {
        return Err(AppError::ToolNotInstalled {
            tool: tool.to_string(),
        });
    }

    Ok(Some(BinResolution { path }))
}

fn bin_path_for_name(
    cache: &Cache,
    tool: &str,
    version: &str,
    bin_name: &str,
    bin_paths: &[String],
) -> Option<PathBuf> {
    let bin_dir = cache.tool_bin_dir(tool, version);
    bin_paths.iter().find_map(|path| {
        let stem = Path::new(path).file_stem()?.to_str()?;
        if stem != bin_name {
            return None;
        }
        let file_name = Path::new(path).file_name()?.to_str()?;
        Some(bin_dir.join(file_name))
    })
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

    fn map(entries: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
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
        let node_bin = cache.tool_bin_dir("node", "22.0.0").join("node");
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
    fn add_shim_creates_file_when_override_is_used() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = add_shim("node", Some(temp.path())).expect("add shim");
        assert!(path.exists());
    }

    #[test]
    fn rebuild_shims_with_empty_config_returns_ok() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = Config::default();
        let created = rebuild_shims(&config, Some(temp.path())).expect("rebuild");
        assert!(created.is_empty());
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
}
