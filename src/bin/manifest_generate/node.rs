use std::collections::HashMap;

use semver::Version;
use serde::Deserialize;

use crate::common::{
    default_targets, fetch_text, TargetSpec, ToolEntry, ToolManifest, ToolVersion,
};

const NODE_MAJORS: &[u64] = &[25, 24, 22, 20, 18];

#[derive(Debug, Deserialize)]
struct NodeIndexEntry {
    version: String,
}

pub(crate) fn generate_node_manifest(generated_at: &str) -> Result<ToolManifest, String> {
    let versions = fetch_node_versions()?;
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

fn fetch_node_versions() -> Result<Vec<Version>, String> {
    let url = "https://nodejs.org/dist/index.json";
    let text = fetch_text(url)?;
    let entries: Vec<NodeIndexEntry> =
        serde_json::from_str(&text).map_err(|err| err.to_string())?;
    let mut seen = HashMap::new();

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

        seen.insert(parsed.to_string(), parsed);
    }

    let mut resolved: Vec<Version> = seen.into_values().collect();
    resolved.sort_unstable_by(|a, b| b.cmp(a));

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

fn node_targets() -> Vec<TargetSpec> {
    default_targets()
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
