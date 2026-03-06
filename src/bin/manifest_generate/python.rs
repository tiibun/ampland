use std::collections::HashMap;

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
    let selected_versions = select_python_versions(&version_map);
    if selected_versions.is_empty() {
        return Err("no python versions with all targets found".to_string());
    }
    let targets = python_targets();
    eprintln!(
        "python: selected {} versions across {} targets",
        selected_versions.len(),
        targets.len()
    );

    for (version_index, selected) in selected_versions.iter().enumerate() {
        eprintln!(
            "python: processing {} ({}/{}, {} targets)",
            selected.version,
            version_index + 1,
            selected_versions.len(),
            targets.len()
        );
        for target in &targets {
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
                bin_paths: python_bin_paths(*target),
            });
        }
    }

    let default_version = selected_versions
        .first()
        .map(|selected| selected.version.to_string())
        .ok_or_else(|| "no python versions resolved".to_string())?;

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

#[derive(Debug)]
struct SelectedPythonVersion {
    version: Version,
    assets: HashMap<(String, String), PythonAssetInfo>,
}

fn select_python_versions(
    version_map: &HashMap<Version, HashMap<(String, String), PythonAssetInfo>>,
) -> Vec<SelectedPythonVersion> {
    let mut selected = Vec::new();

    for (version, assets) in version_map {
        if version.major != 3 {
            continue;
        }
        if !python_version_has_all_targets(assets) {
            continue;
        }

        selected.push(SelectedPythonVersion {
            version: version.clone(),
            assets: assets.clone(),
        });
    }

    selected.sort_unstable_by(|a, b| b.version.cmp(&a.version));
    selected
}

fn python_version_has_all_targets(assets: &HashMap<(String, String), PythonAssetInfo>) -> bool {
    for target in python_targets() {
        let key = (target.platform.to_string(), target.arch.to_string());
        if !assets.contains_key(&key) {
            return false;
        }
    }
    true
}

fn fetch_python_assets() -> Result<Vec<PythonAssetInfo>, String> {
    let url = "https://api.github.com/repos/astral-sh/python-build-standalone/releases?per_page=5";
    eprintln!("python: fetching release metadata");
    let text = fetch_text(url)?;
    let releases: Vec<GithubRelease> =
        serde_json::from_str(&text).map_err(|err| err.to_string())?;
    let release_count = releases.len();
    eprintln!("python: inspecting {release_count} GitHub releases");
    let mut assets = Vec::new();

    for (index, release) in releases.into_iter().enumerate() {
        eprintln!(
            "python: scanning release {} of {}",
            index + 1,
            release_count
        );
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

    eprintln!("python: discovered {} candidate assets", assets.len());
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use semver::Version;

    use super::{python_targets, select_python_versions, PythonAssetInfo};

    #[test]
    fn select_python_versions_keeps_all_complete_patch_releases_descending() {
        let mut version_map = HashMap::new();
        version_map.insert(
            Version::parse("3.13.2").unwrap(),
            assets_for_version("3.13.2"),
        );
        version_map.insert(
            Version::parse("3.13.1").unwrap(),
            assets_for_version("3.13.1"),
        );
        version_map.insert(
            Version::parse("3.12.9").unwrap(),
            assets_for_version("3.12.9"),
        );
        version_map.insert(
            Version::parse("3.12.8").unwrap(),
            incomplete_assets_for_version("3.12.8"),
        );
        version_map.insert(
            Version::parse("3.11.11").unwrap(),
            assets_for_version("3.11.11"),
        );
        version_map.insert(
            Version::parse("2.7.18").unwrap(),
            assets_for_version("2.7.18"),
        );

        let selected = select_python_versions(&version_map);
        let version_strings: Vec<String> = selected
            .into_iter()
            .map(|version| version.version.to_string())
            .collect();

        assert_eq!(
            version_strings,
            vec!["3.13.2", "3.13.1", "3.12.9", "3.11.11"]
        );
    }

    fn assets_for_version(version: &str) -> HashMap<(String, String), PythonAssetInfo> {
        python_targets()
            .into_iter()
            .map(|target| {
                let key = (target.platform.to_string(), target.arch.to_string());
                let info = PythonAssetInfo {
                    version: Version::parse(version).unwrap(),
                    version_str: version.to_string(),
                    target,
                    url: format!("https://example.com/{version}/{}-{}", key.0, key.1),
                    sha256_url: Some(format!(
                        "https://example.com/{version}/{}-{}.sha256",
                        key.0, key.1
                    )),
                };
                (key, info)
            })
            .collect()
    }

    fn incomplete_assets_for_version(version: &str) -> HashMap<(String, String), PythonAssetInfo> {
        let mut assets = assets_for_version(version);
        assets.remove(&(String::from("windows"), String::from("x64")));
        assets
    }
}
