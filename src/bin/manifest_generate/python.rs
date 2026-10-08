use std::collections::HashMap;

use semver::Version;
use serde::Deserialize;

use crate::common::{
    default_targets, fetch_sha256, fetch_text, TargetSpec, ToolEntry, ToolManifest, ToolVersion,
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

            let sha256 = resolve_python_sha256(asset)?;

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

    let prefix_len = "cpython-".len();
    let suffix_len = "-install_only.tar.gz".len();
    if asset.name.len() < prefix_len + suffix_len {
        return Ok(None);
    }

    let trimmed = &asset.name[prefix_len..asset.name.len() - suffix_len];
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

fn resolve_python_sha256(asset: &PythonAssetInfo) -> Result<String, String> {
    match &asset.sha256_url {
        Some(url) => fetch_sha256(url),
        None => Err(format!(
            "no .sha256 asset published for {} ({} {}); refusing to fall back to hashing the \
             downloaded artifact, which provides no integrity protection against a compromised \
             upstream",
            asset.url, asset.target.platform, asset.target.arch
        )),
    }
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

    use super::{
        parse_python_asset, python_targets, resolve_python_sha256, select_python_versions,
        GithubAsset, PythonAssetInfo,
    };

    #[test]
    fn resolve_python_sha256_errors_when_no_sha256_asset_is_published() {
        let target = python_targets().remove(0);
        let asset = PythonAssetInfo {
            version: Version::parse("3.13.2").unwrap(),
            version_str: "3.13.2".to_string(),
            target,
            url: "https://example.com/cpython-3.13.2.tar.gz".to_string(),
            sha256_url: None,
        };

        let result = resolve_python_sha256(&asset);

        assert!(result.is_err());
        let message = result.unwrap_err();
        assert!(message.contains("cpython-3.13.2.tar.gz"));
    }

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

    fn asset_named(name: &str) -> GithubAsset {
        GithubAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
        }
    }

    #[test]
    fn parse_python_asset_does_not_panic_on_exact_exploit_name() {
        // "cpython-" (8) + "-install_only.tar.gz" (20) overlap by one byte in this
        // name (len 27), so the unchecked slice would previously panic.
        let asset = asset_named("cpython-install_only.tar.gz");
        let result = parse_python_asset(&asset, &HashMap::new()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_python_asset_handles_exact_boundary_length_with_empty_middle() {
        // len == prefix_len + suffix_len exactly (28); trimmed middle is empty,
        // which is not a valid version/triple, so this should resolve to None
        // rather than panicking.
        let asset = asset_named("cpython--install_only.tar.gz");
        let result = parse_python_asset(&asset, &HashMap::new()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_python_asset_handles_one_byte_above_boundary() {
        let asset = asset_named("cpython-x-install_only.tar.gz");
        let result = parse_python_asset(&asset, &HashMap::new()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_python_asset_parses_well_formed_name() {
        let name = "cpython-3.12.9+20250205-x86_64-unknown-linux-gnu-install_only.tar.gz";
        let asset = asset_named(name);
        let info = parse_python_asset(&asset, &HashMap::new())
            .unwrap()
            .expect("well-formed asset name should parse");
        assert_eq!(info.version_str, "3.12.9");
        assert_eq!(info.target.platform, "linux");
        assert_eq!(info.target.arch, "x64");
        assert_eq!(info.url, asset.browser_download_url);
    }
}
