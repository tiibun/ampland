use std::collections::{HashMap, HashSet};

use semver::Version;
use serde::Deserialize;

use crate::common::{
    default_targets, download_and_hash, fetch_sha256, fetch_text, TargetSpec, ToolEntry,
    ToolManifest, ToolVersion,
};

#[derive(Debug, Deserialize)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone)]
struct PythonAssetInfo {
    version: Version,
    version_str: String,
    target: TargetSpec,
    url: String,
    sha256_url: Option<String>,
}

pub(crate) fn generate_python_manifest(generated_at: &str) -> Result<ToolManifest, String> {
    let assets = fetch_python_assets()?;
    let mut version_map: HashMap<Version, HashMap<(String, String), PythonAssetInfo>> =
        HashMap::new();

    for asset in assets {
        let target_key = (
            asset.target.platform.to_string(),
            asset.target.arch.to_string(),
        );
        version_map
            .entry(asset.version.clone())
            .or_default()
            .insert(target_key, asset);
    }

    let mut tool_versions = Vec::new();
    let mut resolved_versions = Vec::new();

    let minors = python_minors_with_all_targets(&version_map);
    if minors.is_empty() {
        return Err("no python versions with all targets found".to_string());
    }

    for minor in minors {
        let selected = select_python_version(&version_map, minor)?;
        let version_str = selected.version.to_string();
        resolved_versions.push(version_str.clone());

        for target in python_targets() {
            let target_key = (target.platform.to_string(), target.arch.to_string());
            let asset = selected.assets.get(&target_key).ok_or_else(|| {
                format!(
                    "missing python asset for {} {} {}",
                    selected.version, target.platform, target.arch
                )
            })?;

            let sha256 = match &asset.sha256_url {
                Some(url) => fetch_sha256(url)?,
                None => download_and_hash(&asset.url)?,
            };

            tool_versions.push(ToolVersion {
                ver: asset.version_str.clone(),
                platform: target.platform.to_string(),
                arch: target.arch.to_string(),
                url: asset.url.clone(),
                sha256,
                format: "tar.gz".to_string(),
                bin_paths: python_bin_paths(target),
            });
        }
    }

    let default_version = resolved_versions
        .first()
        .ok_or_else(|| "no python versions resolved".to_string())?
        .to_string();

    Ok(ToolManifest {
        version: 1,
        generated_at: generated_at.to_string(),
        tools: vec![ToolEntry {
            name: "python".to_string(),
            vendor: "cpython".to_string(),
            default_version,
            versions: tool_versions,
        }],
    })
}

fn python_minors_with_all_targets(
    version_map: &HashMap<Version, HashMap<(String, String), PythonAssetInfo>>,
) -> Vec<u64> {
    let targets = python_targets();
    let mut minors = HashSet::new();

    for (version, assets) in version_map {
        if version.major != 3 {
            continue;
        }
        let mut missing = false;
        for target in &targets {
            let key = (target.platform.to_string(), target.arch.to_string());
            if !assets.contains_key(&key) {
                missing = true;
                break;
            }
        }
        if !missing {
            minors.insert(version.minor);
        }
    }

    let mut minors: Vec<u64> = minors.into_iter().collect();
    minors.sort_unstable_by(|a, b| b.cmp(a));
    minors
}

#[derive(Debug)]
struct SelectedPythonVersion {
    version: Version,
    assets: HashMap<(String, String), PythonAssetInfo>,
}

fn select_python_version(
    version_map: &HashMap<Version, HashMap<(String, String), PythonAssetInfo>>,
    minor: u64,
) -> Result<SelectedPythonVersion, String> {
    let targets = python_targets();
    let mut best: Option<Version> = None;

    for version in version_map.keys() {
        if version.major != 3 || version.minor != minor {
            continue;
        }
        let assets = match version_map.get(version) {
            Some(value) => value,
            None => continue,
        };
        let mut missing = false;
        for target in &targets {
            let key = (target.platform.to_string(), target.arch.to_string());
            if !assets.contains_key(&key) {
                missing = true;
                break;
            }
        }
        if missing {
            continue;
        }
        let update = match &best {
            Some(current) => version > current,
            None => true,
        };
        if update {
            best = Some(version.clone());
        }
    }

    let version = best.ok_or_else(|| format!("no python {minor}.x release with all targets"))?;
    let assets = version_map
        .get(&version)
        .cloned()
        .ok_or_else(|| "selected python version missing assets".to_string())?;

    Ok(SelectedPythonVersion { version, assets })
}

fn fetch_python_assets() -> Result<Vec<PythonAssetInfo>, String> {
    let url = "https://api.github.com/repos/astral-sh/python-build-standalone/releases?per_page=5";
    let text = fetch_text(url)?;
    let releases: Vec<GithubRelease> =
        serde_json::from_str(&text).map_err(|err| err.to_string())?;
    let mut assets = Vec::new();

    for release in releases {
        let mut sha256_by_name = HashMap::new();
        for asset in &release.assets {
            if asset.name.ends_with(".sha256") || asset.name.ends_with(".sha256.txt") {
                sha256_by_name.insert(asset.name.clone(), asset.browser_download_url.clone());
            }
        }

        for asset in &release.assets {
            if let Some(info) = parse_python_asset(asset, &sha256_by_name)? {
                assets.push(info);
            }
        }
    }

    Ok(assets)
}

fn parse_python_asset(
    asset: &GithubAsset,
    sha256_by_name: &HashMap<String, String>,
) -> Result<Option<PythonAssetInfo>, String> {
    if !asset.name.starts_with("cpython-") {
        return Ok(None);
    }
    if !asset.name.ends_with("-install_only.tar.gz") {
        return Ok(None);
    }

    let trimmed = &asset.name["cpython-".len()..asset.name.len() - "-install_only.tar.gz".len()];
    let triples = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "x86_64-pc-windows-msvc",
    ];

    let (version_date, triple) = match triples.iter().find_map(|triple| {
        let suffix = format!("-{triple}");
        trimmed.strip_suffix(&suffix).map(|value| (value, *triple))
    }) {
        Some(value) => value,
        None => return Ok(None),
    };

    let (version_str, _date) = match version_date.split_once('+') {
        Some(value) => value,
        None => return Ok(None),
    };

    let version = match Version::parse(version_str) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };

    let target = match python_target_from_triple(triple) {
        Some(value) => value,
        None => return Ok(None),
    };

    let sha256_url = sha256_by_name
        .get(&format!("{}.sha256", asset.name))
        .or_else(|| sha256_by_name.get(&format!("{}.sha256.txt", asset.name)))
        .cloned();

    Ok(Some(PythonAssetInfo {
        version,
        version_str: version_str.to_string(),
        target,
        url: asset.browser_download_url.clone(),
        sha256_url,
    }))
}

fn python_targets() -> Vec<TargetSpec> {
    default_targets()
}

fn python_target_from_triple(triple: &str) -> Option<TargetSpec> {
    match triple {
        "aarch64-apple-darwin" => Some(TargetSpec {
            platform: "macos",
            arch: "arm64",
        }),
        "x86_64-apple-darwin" => Some(TargetSpec {
            platform: "macos",
            arch: "x64",
        }),
        "aarch64-unknown-linux-gnu" => Some(TargetSpec {
            platform: "linux",
            arch: "arm64",
        }),
        "x86_64-unknown-linux-gnu" => Some(TargetSpec {
            platform: "linux",
            arch: "x64",
        }),
        "x86_64-pc-windows-msvc" => Some(TargetSpec {
            platform: "windows",
            arch: "x64",
        }),
        _ => None,
    }
}

fn python_bin_paths(target: TargetSpec) -> Vec<String> {
    match target.platform {
        "windows" => vec!["python/python.exe".to_string()],
        _ => vec![
            "python/bin/python".to_string(),
            "python/bin/python3".to_string(),
        ],
    }
}