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
    let targets = node_targets();
    eprintln!(
        "node: selected {} versions across {} targets",
        versions.len(),
        targets.len()
    );
    let mut tool_versions = Vec::new();

    for (index, version) in versions.iter().enumerate() {
        if should_log_node_progress(index, versions.len()) {
            eprintln!(
                "node: fetching checksums for {version} ({}/{})",
                index + 1,
                versions.len()
            );
        }
        let sha256s = fetch_node_sha256s(&version.to_string())?;
        for target in &targets {
            let filename = node_filename(&version.to_string(), *target)?;
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
                format: node_format(*target).to_string(),
                bin_paths: node_bin_paths(&version.to_string(), *target),
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
    eprintln!("node: fetching release index");
    let text = fetch_text(url)?;
    let entries: Vec<NodeIndexEntry> =
        serde_json::from_str(&text).map_err(|err| err.to_string())?;
    let versions = select_node_versions(entries);
    eprintln!(
        "node: release index resolved to {} versions",
        versions.len()
    );
    Ok(versions)
}

fn select_node_versions(entries: Vec<NodeIndexEntry>) -> Vec<Version> {
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

    resolved
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

fn should_log_node_progress(index: usize, total: usize) -> bool {
    index == 0 || index + 1 == total || (index + 1).is_multiple_of(10)
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

#[cfg(test)]
mod tests {
    use super::{select_node_versions, should_log_node_progress, NodeIndexEntry};

    #[test]
    fn select_node_versions_keeps_all_matching_releases_descending() {
        let versions = select_node_versions(vec![
            NodeIndexEntry {
                version: "v24.1.0".to_string(),
            },
            NodeIndexEntry {
                version: "v24.0.0".to_string(),
            },
            NodeIndexEntry {
                version: "v25.0.0".to_string(),
            },
            NodeIndexEntry {
                version: "v20.18.1".to_string(),
            },
            NodeIndexEntry {
                version: "v19.9.0".to_string(),
            },
            NodeIndexEntry {
                version: "v24.1.0-rc.1".to_string(),
            },
            NodeIndexEntry {
                version: "invalid".to_string(),
            },
            NodeIndexEntry {
                version: "v24.1.0".to_string(),
            },
        ]);

        let version_strings: Vec<String> = versions
            .into_iter()
            .map(|version| version.to_string())
            .collect();
        assert_eq!(
            version_strings,
            vec!["25.0.0", "24.1.0", "24.0.0", "20.18.1"]
        );
    }

    #[test]
    fn logs_node_progress_at_milestones() {
        assert!(should_log_node_progress(0, 139));
        assert!(should_log_node_progress(9, 139));
        assert!(!should_log_node_progress(10, 139));
        assert!(should_log_node_progress(138, 139));
    }
}
