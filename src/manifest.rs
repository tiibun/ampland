use std::collections::HashSet;
use semver::Version;
use serde::Deserialize;

use crate::error::AppError;
const EMBEDDED_TOOL_MANIFESTS: &[&str] = &[
    include_str!("../assets/manifest/node.toml"),
    include_str!("../assets/manifest/python.toml"),
];

#[derive(Debug, Clone)]
pub struct Target {
    pub platform: String,
    pub arch: String,
}

impl Target {
    pub fn current() -> Result<Self, AppError> {
        let platform = match std::env::consts::OS {
            "macos" => "macos",
            "linux" => "linux",
            "windows" => "windows",
            other => {
                return Err(AppError::Other {
                    message: format!("unsupported platform: {other}"),
                })
            }
        }
        .to_string();

        let arch = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            other => {
                return Err(AppError::Other {
                    message: format!("unsupported arch: {other}"),
                })
            }
        }
        .to_string();

        Ok(Target { platform, arch })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub generated_at: String,
    #[serde(default, rename = "tool")]
    pub tools: Vec<ManifestTool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestTool {
    pub name: String,
    #[serde(default, rename = "version")]
    pub versions: Vec<ManifestToolVersion>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestToolVersion {
    pub ver: String,
    pub platform: String,
    pub arch: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default = "default_format")]
    pub format: String,
    #[serde(default)]
    pub bin_path: Option<String>,
    #[serde(default)]
    pub bin_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    pub url: String,
    pub sha256: String,
    pub size: Option<u64>,
    pub format: PackageFormat,
    pub bin_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageFormat {
    File,
    TarGz,
    TarXz,
    Zip,
}

impl Manifest {
    pub fn parse(contents: &str) -> Result<Self, AppError> {
        Ok(toml::from_str(contents)?)
    }

    pub fn resolve_version_spec(
        &self,
        tool: &str,
        version_spec: &str,
        target: &Target,
    ) -> Option<String> {
        let tool_entry = self.tools.iter().find(|entry| entry.name == tool)?;
        if let Some(entry) = tool_entry.versions.iter().find(|entry| {
            entry.platform == target.platform
                && entry.arch == target.arch
                && entry.ver == version_spec
        }) {
            return Some(entry.ver.clone());
        }

        let normalized_spec = version_spec.trim_start_matches('v');
        if normalized_spec != version_spec {
            if let Some(entry) = tool_entry.versions.iter().find(|entry| {
                entry.platform == target.platform
                    && entry.arch == target.arch
                    && entry.ver == normalized_spec
            }) {
                return Some(entry.ver.clone());
            }
        }

        let parts: Vec<&str> = normalized_spec.split('.').collect();
        if parts.len() != 1 && parts.len() != 2 {
            return None;
        }

        let major = parts[0].parse::<u64>().ok()?;
        let minor = if parts.len() == 2 {
            Some(parts[1].parse::<u64>().ok()?)
        } else {
            None
        };

        let mut best: Option<(Version, String)> = None;
        for entry in tool_entry
            .versions
            .iter()
            .filter(|entry| entry.platform == target.platform && entry.arch == target.arch)
        {
            let parsed = match Version::parse(entry.ver.trim_start_matches('v')) {
                Ok(value) => value,
                Err(_) => continue,
            };

            if parsed.major != major {
                continue;
            }
            if let Some(minor) = minor {
                if parsed.minor != minor {
                    continue;
                }
            }

            match &best {
                Some((best_version, _)) if &parsed <= best_version => {}
                _ => best = Some((parsed, entry.ver.clone())),
            }
        }

        best.map(|(_, ver)| ver)
    }

    pub fn resolve(&self, tool: &str, version: &str, target: &Target) -> Option<ResolvedPackage> {
        let tool_entry = self.tools.iter().find(|entry| entry.name == tool)?;
        let version_entry = tool_entry.versions.iter().find(|entry| {
            entry.ver == version && entry.platform == target.platform && entry.arch == target.arch
        })?;

        let format = match version_entry.format.as_str() {
            "file" => PackageFormat::File,
            "tar.gz" => PackageFormat::TarGz,
            "tar.xz" => PackageFormat::TarXz,
            "zip" => PackageFormat::Zip,
            _ => return None,
        };

        Some(ResolvedPackage {
            url: version_entry.url.clone(),
            sha256: version_entry.sha256.clone(),
            size: version_entry.size,
            format,
            bin_paths: resolve_bin_paths(version_entry),
        })
    }
}

fn resolve_bin_paths(version_entry: &ManifestToolVersion) -> Vec<String> {
    if let Some(paths) = &version_entry.bin_paths {
        return paths.clone();
    }
    version_entry.bin_path.clone().into_iter().collect()
}

pub fn load_manifest() -> Result<Manifest, AppError> {
    if EMBEDDED_TOOL_MANIFESTS.is_empty() {
        return Err(AppError::Other {
            message: "embedded manifest list is empty".to_string(),
        });
    }

    let mut manifests = Vec::with_capacity(EMBEDDED_TOOL_MANIFESTS.len());
    for manifest_text in EMBEDDED_TOOL_MANIFESTS {
        manifests.push(Manifest::parse(manifest_text)?);
    }

    let mut iter = manifests.into_iter();
    let mut merged = iter.next().ok_or_else(|| AppError::Other {
        message: "embedded manifest list is empty".to_string(),
    })?;

    let mut seen = HashSet::new();
    for tool in &merged.tools {
        seen.insert(tool.name.clone());
    }

    for manifest in iter {
        if manifest.version != merged.version {
            return Err(AppError::Other {
                message: "embedded manifest version mismatch".to_string(),
            });
        }
        if manifest.generated_at > merged.generated_at {
            merged.generated_at = manifest.generated_at.clone();
        }
        for tool in manifest.tools {
            if !seen.insert(tool.name.clone()) {
                return Err(AppError::Other {
                    message: format!("duplicate tool in embedded manifests: {}", tool.name),
                });
            }
            merged.tools.push(tool);
        }
    }

    Ok(merged)
}

fn default_format() -> String {
    "file".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> Manifest {
        let text = r#"
version = 1
generated_at = "2026-02-11T00:00:00Z"

[[tool]]
name = "node"

  [[tool.version]]
  ver = "22.3.0"
  platform = "macos"
  arch = "arm64"
  url = "https://example.com/node.tar.gz"
  sha256 = "deadbeef"
  format = "tar.gz"
    bin_paths = ["node/bin/node", "node/bin/npm", "node/bin/npx"]

  [[tool.version]]
  ver = "22.3.0"
  platform = "linux"
  arch = "x64"
  url = "https://example.com/node"
  sha256 = "beadbead"
"#;

        Manifest::parse(text).expect("manifest parse")
    }

    #[test]
    fn resolves_matching_entry() {
        let manifest = sample_manifest();
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        let resolved = manifest
            .resolve("node", "22.3.0", &target)
            .expect("resolve entry");
        assert_eq!(resolved.url, "https://example.com/node.tar.gz");
        assert_eq!(resolved.sha256, "deadbeef");
        assert_eq!(resolved.format, PackageFormat::TarGz);
        assert_eq!(
            resolved.bin_paths,
            vec!["node/bin/node", "node/bin/npm", "node/bin/npx"]
        );
    }

    #[test]
    fn defaults_format_to_file() {
        let manifest = sample_manifest();
        let target = Target {
            platform: "linux".to_string(),
            arch: "x64".to_string(),
        };
        let resolved = manifest
            .resolve("node", "22.3.0", &target)
            .expect("resolve entry");
        assert_eq!(resolved.format, PackageFormat::File);
    }

    #[test]
    fn returns_none_for_unknown_format() {
        let text = r#"
version = 1
generated_at = "2026-02-11T00:00:00Z"

[[tool]]
name = "node"

  [[tool.version]]
  ver = "22.3.0"
  platform = "macos"
  arch = "arm64"
  url = "https://example.com/node.zip"
  sha256 = "deadbeef"
    format = "rar"
"#;
        let manifest = Manifest::parse(text).expect("manifest parse");
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        let resolved = manifest.resolve("node", "22.3.0", &target);
        assert!(resolved.is_none());
    }

    #[test]
    fn resolves_version_specs_for_exact_prefixed_and_partial() {
        let manifest = sample_manifest();
        let target = Target {
            platform: "macos".to_string(),
            arch: "arm64".to_string(),
        };
        assert_eq!(
            manifest.resolve_version_spec("node", "22.3.0", &target),
            Some("22.3.0".to_string())
        );
        assert_eq!(
            manifest.resolve_version_spec("node", "v22.3.0", &target),
            Some("22.3.0".to_string())
        );
        assert_eq!(
            manifest.resolve_version_spec("node", "22", &target),
            Some("22.3.0".to_string())
        );
        assert_eq!(manifest.resolve_version_spec("node", "23", &target), None);
    }

    #[test]
    fn default_format_is_file() {
        assert_eq!(default_format(), "file");
    }
}
