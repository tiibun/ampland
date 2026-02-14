use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const USER_AGENT: &str = "ampland-manifest-generate";
const OUTPUT_DIR_DEFAULT: &str = "assets/manifest";

const NODE_MAJORS: &[u64] = &[24, 22, 20, 18];
const PYTHON_MINORS: &[u64] = &[14, 13, 12];

fn main() {
    if let Err(err) = run() {
        eprintln!("manifest-generate: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let output_dir = parse_args()?;
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|err| err.to_string())?;

    let node_manifest = generate_node_manifest(&generated_at)?;
    write_manifest(&output_dir.join("node.toml"), &node_manifest)?;

    let python_manifest = generate_python_manifest(&generated_at)?;
    write_manifest(&output_dir.join("python.toml"), &python_manifest)?;

    println!(
        "Wrote {} and {}",
        output_dir.join("node.toml").display(),
        output_dir.join("python.toml").display()
    );

    Ok(())
}

fn parse_args() -> Result<PathBuf, String> {
    let mut args = env::args().skip(1);
    let mut output_dir = None;

    while let Some(arg) = args.next() {
        if arg == "--output-dir" {
            let value = args
                .next()
                .ok_or_else(|| "missing value for --output-dir".to_string())?;
            output_dir = Some(PathBuf::from(value));
        } else {
            return Err(format!("unknown argument: {arg}"));
        }
    }

    Ok(output_dir.unwrap_or_else(|| PathBuf::from(OUTPUT_DIR_DEFAULT)))
}

#[derive(Debug, Serialize)]
struct ToolManifest {
    version: u32,
    generated_at: String,
    #[serde(rename = "tool")]
    tools: Vec<ToolEntry>,
}

#[derive(Debug, Serialize)]
struct ToolEntry {
    name: String,
    vendor: String,
    default_version: String,
    #[serde(rename = "version")]
    versions: Vec<ToolVersion>,
}

#[derive(Debug, Serialize)]
struct ToolVersion {
    ver: String,
    platform: String,
    arch: String,
    url: String,
    sha256: String,
    format: String,
    bin_paths: Vec<String>,
}

fn write_manifest(path: &Path, manifest: &ToolManifest) -> Result<(), String> {
    let output = toml::to_string_pretty(manifest).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    fs::write(path, output).map_err(|err| err.to_string())?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct NodeIndexEntry {
    version: String,
}

fn generate_node_manifest(generated_at: &str) -> Result<ToolManifest, String> {
    let versions = fetch_latest_node_versions()?;
    let mut tool_versions = Vec::new();

    for version in &versions {
        let sha256s = fetch_node_sha256s(&version.to_string())?;
        for target in node_targets() {
            let filename = node_filename(&version.to_string(), target)?;
            let sha256 = sha256s
                .get(&filename)
                .ok_or_else(|| format!("missing sha256 for {filename}"))?
                .to_string();
            let url = format!("https://nodejs.org/dist/v{version}/{filename}");
            tool_versions.push(ToolVersion {
                ver: version.to_string(),
                platform: target.platform.to_string(),
                arch: target.arch.to_string(),
                url,
                sha256,
                format: node_format(target).to_string(),
                bin_paths: node_bin_paths(&version.to_string(), target),
            });
        }
    }

    let default_version = versions
        .first()
        .map(|ver| ver.to_string())
        .ok_or_else(|| "no node versions resolved".to_string())?;

    Ok(ToolManifest {
        version: 1,
        generated_at: generated_at.to_string(),
        tools: vec![ToolEntry {
            name: "node".to_string(),
            vendor: "nodejs".to_string(),
            default_version,
            versions: tool_versions,
        }],
    })
}

fn fetch_latest_node_versions() -> Result<Vec<Version>, String> {
    let url = "https://nodejs.org/dist/index.json";
    let text = fetch_text(url)?;
    let entries: Vec<NodeIndexEntry> =
        serde_json::from_str(&text).map_err(|err| err.to_string())?;
    let mut by_major: HashMap<u64, Version> = HashMap::new();

    for entry in entries {
        let trimmed = entry.version.trim_start_matches('v');
        let parsed = match Version::parse(trimmed) {
            Ok(version) => version,
            Err(_) => continue,
        };
        if !parsed.pre.is_empty() {
            continue;
        }
        if !NODE_MAJORS.contains(&parsed.major) {
            continue;
        }

        let update = match by_major.get(&parsed.major) {
            Some(current) => &parsed > current,
            None => true,
        };
        if update {
            by_major.insert(parsed.major, parsed);
        }
    }

    let mut resolved = Vec::new();
    for major in NODE_MAJORS {
        let version = by_major
            .get(major)
            .ok_or_else(|| format!("no node versions found for major {major}"))?;
        resolved.push(version.clone());
    }

    Ok(resolved)
}

fn fetch_node_sha256s(version: &str) -> Result<HashMap<String, String>, String> {
    let url = format!("https://nodejs.org/dist/v{version}/SHASUMS256.txt");
    let text = fetch_text(&url)?;
    let mut map = HashMap::new();

    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let hash = match parts.next() {
            Some(value) => value,
            None => continue,
        };
        let filename = match parts.next() {
            Some(value) => value,
            None => continue,
        };
        map.insert(filename.to_string(), hash.to_string());
    }

    Ok(map)
}

#[derive(Debug, Clone, Copy)]
struct TargetSpec {
    platform: &'static str,
    arch: &'static str,
}

fn node_targets() -> Vec<TargetSpec> {
    vec![
        TargetSpec {
            platform: "macos",
            arch: "arm64",
        },
        TargetSpec {
            platform: "macos",
            arch: "x64",
        },
        TargetSpec {
            platform: "linux",
            arch: "arm64",
        },
        TargetSpec {
            platform: "linux",
            arch: "x64",
        },
        TargetSpec {
            platform: "windows",
            arch: "x64",
        },
    ]
}

fn node_filename(version: &str, target: TargetSpec) -> Result<String, String> {
    let suffix = match (target.platform, target.arch) {
        ("macos", "arm64") => "darwin-arm64.tar.gz",
        ("macos", "x64") => "darwin-x64.tar.gz",
        ("linux", "arm64") => "linux-arm64.tar.xz",
        ("linux", "x64") => "linux-x64.tar.xz",
        ("windows", "x64") => "win-x64.zip",
        _ => {
            return Err(format!(
                "unsupported node target: {}-{}",
                target.platform, target.arch
            ))
        }
    };

    Ok(format!("node-v{version}-{suffix}"))
}

fn node_format(target: TargetSpec) -> &'static str {
    match (target.platform, target.arch) {
        ("macos", "arm64") | ("macos", "x64") => "tar.gz",
        ("linux", "arm64") | ("linux", "x64") => "tar.xz",
        ("windows", "x64") => "zip",
        _ => "zip",
    }
}

fn node_bin_paths(version: &str, target: TargetSpec) -> Vec<String> {
    let base = match (target.platform, target.arch) {
        ("macos", "arm64") => format!("node-v{version}-darwin-arm64"),
        ("macos", "x64") => format!("node-v{version}-darwin-x64"),
        ("linux", "arm64") => format!("node-v{version}-linux-arm64"),
        ("linux", "x64") => format!("node-v{version}-linux-x64"),
        ("windows", "x64") => format!("node-v{version}-win-x64"),
        _ => format!("node-v{version}"),
    };

    match target.platform {
        "windows" => vec![
            format!("{base}/node.exe"),
            format!("{base}/npm.cmd"),
            format!("{base}/npx.cmd"),
        ],
        _ => vec![
            format!("{base}/bin/node"),
            format!("{base}/bin/npm"),
            format!("{base}/bin/npx"),
        ],
    }
}

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

fn generate_python_manifest(generated_at: &str) -> Result<ToolManifest, String> {
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

    for minor in PYTHON_MINORS {
        let selected = select_python_version(&version_map, *minor)?;
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
    let url = "https://api.github.com/repos/astral-sh/python-build-standalone/releases?per_page=20";
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
    node_targets()
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

fn fetch_text(url: &str) -> Result<String, String> {
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|err| err.to_string())?;
    response.into_string().map_err(|err| err.to_string())
}

fn fetch_sha256(url: &str) -> Result<String, String> {
    let text = fetch_text(url)?;
    let hash = text
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("empty sha256 response from {url}"))?;
    Ok(hash.to_string())
}

fn download_and_hash(url: &str) -> Result<String, String> {
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|err| err.to_string())?;
    let mut reader = response.into_reader();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = reader.read(&mut buf).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
