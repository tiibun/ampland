use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use fs2::FileExt;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::config::ManifestConfig;
use crate::error::AppError;

const DEFAULT_PUBLIC_KEY_HEX: &str = "";
const DEFAULT_TTL_HOURS: u64 = 24;
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
        for entry in tool_entry.versions.iter().filter(|entry| {
            entry.platform == target.platform && entry.arch == target.arch
        }) {
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
            entry.ver == version
                && entry.platform == target.platform
                && entry.arch == target.arch
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
    version_entry
        .bin_path
        .clone()
        .into_iter()
        .collect()
}

#[derive(Debug, Clone)]
pub struct ManifestStore {
    cache_root: PathBuf,
    config: ManifestConfig,
}

impl ManifestStore {
    pub fn new(cache_root: &Path, config: &ManifestConfig) -> Self {
        ManifestStore {
            cache_root: cache_root.to_path_buf(),
            config: config.clone(),
        }
    }

    pub fn load(&self) -> Result<Manifest, AppError> {
        let embedded = embedded_manifest()?;
        let ttl = Duration::from_secs(self.ttl_hours() * 3600);
        let public_key = self.resolve_public_key()?;
        let cached = self.read_cached(public_key.as_deref())?;

        if let Some(cache) = &cached {
            if !cache.is_stale(ttl)? {
                return Ok(cache.manifest.clone());
            }
        }

        if let Some(updated) = self.try_update(public_key.as_deref())? {
            return Ok(updated);
        }

        if let Some(cache) = cached {
            return Ok(cache.manifest);
        }

        Ok(embedded)
    }

    pub fn refresh(&self) -> Result<Manifest, AppError> {
        let url = self.config.url.as_ref().ok_or_else(|| AppError::Config {
            message: "manifest.url is required to update".to_string(),
        })?;
        let public_key = self.resolve_public_key()?.ok_or_else(|| AppError::Config {
            message: "manifest.public_key is required to update".to_string(),
        })?;
        let sig_url = self
            .config
            .sig_url
            .clone()
            .unwrap_or_else(|| format!("{url}.sig"));

        let manifest_text = fetch_text(url)?;
        let sig_text = fetch_text(&sig_url)?;
        verify_signature(&public_key, manifest_text.as_bytes(), sig_text.trim())?;
        let manifest = Manifest::parse(&manifest_text)?;
        self.write_cache(&manifest_text, sig_text.trim(), &manifest)?;
        Ok(manifest)
    }

    fn ttl_hours(&self) -> u64 {
        self.config.ttl_hours.unwrap_or(DEFAULT_TTL_HOURS)
    }

    fn try_update(&self, public_key: Option<&[u8]>) -> Result<Option<Manifest>, AppError> {
        let url = match &self.config.url {
            Some(url) => url.clone(),
            None => return Ok(None),
        };

        let public_key = match public_key {
            Some(key) => key,
            None => return Ok(None),
        };

        let sig_url = self
            .config
            .sig_url
            .clone()
            .unwrap_or_else(|| format!("{url}.sig"));

        let manifest_text = fetch_text(&url)?;
        let sig_text = fetch_text(&sig_url)?;
        verify_signature(public_key, manifest_text.as_bytes(), sig_text.trim())?;
        let manifest = Manifest::parse(&manifest_text)?;
        self.write_cache(&manifest_text, sig_text.trim(), &manifest)?;
        Ok(Some(manifest))
    }

    fn read_cached(&self, public_key: Option<&[u8]>) -> Result<Option<ManifestCache>, AppError> {
        let manifest_path = manifest_toml_path(&self.cache_root);
        let sig_path = manifest_sig_path(&self.cache_root);
        let meta_path = manifest_meta_path(&self.cache_root);

        if !manifest_path.exists() || !sig_path.exists() || !meta_path.exists() {
            return Ok(None);
        }

        let manifest_text = fs::read_to_string(&manifest_path)?;
        let sig_text = fs::read_to_string(&sig_path)?;
        let meta_text = fs::read_to_string(&meta_path)?;
        let meta: ManifestMeta = serde_json::from_str(&meta_text)?;

        if let Some(key) = public_key {
            verify_signature(key, manifest_text.as_bytes(), sig_text.trim())?;
        } else {
            return Ok(None);
        }

        let manifest = Manifest::parse(&manifest_text)?;
        Ok(Some(ManifestCache { manifest, meta }))
    }

    fn write_cache(
        &self,
        manifest_text: &str,
        sig_text: &str,
        manifest: &Manifest,
    ) -> Result<(), AppError> {
        let dir = manifest_cache_dir(&self.cache_root);
        fs::create_dir_all(&dir)?;
        let lock_file = File::create(dir.join(".lock"))?;
        lock_file.lock_exclusive()?;

        let manifest_path = manifest_toml_path(&self.cache_root);
        let sig_path = manifest_sig_path(&self.cache_root);
        let meta_path = manifest_meta_path(&self.cache_root);
        fs::write(&manifest_path, manifest_text)?;
        fs::write(&sig_path, sig_text)?;

        let meta = ManifestMeta {
            fetched_at: current_epoch_secs()?,
            manifest_version: manifest.version,
        };
        let meta_text = serde_json::to_string_pretty(&meta)?;
        let mut file = File::create(meta_path)?;
        file.write_all(meta_text.as_bytes())?;

        lock_file.unlock()?;
        Ok(())
    }

    fn resolve_public_key(&self) -> Result<Option<Vec<u8>>, AppError> {
        if let Some(key) = &self.config.public_key {
            let decoded = decode_hex(key)?;
            if is_zero_key(&decoded) {
                return Ok(None);
            }
            return Ok(Some(decoded));
        }

        if DEFAULT_PUBLIC_KEY_HEX.is_empty() {
            return Ok(None);
        }
        let decoded = decode_hex(DEFAULT_PUBLIC_KEY_HEX)?;
        if is_zero_key(&decoded) {
            return Ok(None);
        }
        Ok(Some(decoded))
    }
}

fn embedded_manifest() -> Result<Manifest, AppError> {
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

#[derive(Debug, Clone)]
struct ManifestCache {
    manifest: Manifest,
    meta: ManifestMeta,
}

impl ManifestCache {
    fn is_stale(&self, ttl: Duration) -> Result<bool, AppError> {
        let fetched = UNIX_EPOCH
            .checked_add(Duration::from_secs(self.meta.fetched_at))
            .ok_or_else(|| AppError::Other {
                message: "invalid manifest timestamp".to_string(),
            })?;
        let now = SystemTime::now();
        Ok(now.duration_since(fetched).unwrap_or_default() > ttl)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestMeta {
    fetched_at: u64,
    manifest_version: u32,
}

fn default_format() -> String {
    "file".to_string()
}

fn manifest_cache_dir(root: &Path) -> PathBuf {
    root.join("manifest")
}

fn manifest_toml_path(root: &Path) -> PathBuf {
    manifest_cache_dir(root).join("installers.toml")
}

fn manifest_sig_path(root: &Path) -> PathBuf {
    manifest_cache_dir(root).join("installers.toml.sig")
}

fn manifest_meta_path(root: &Path) -> PathBuf {
    manifest_cache_dir(root).join("installers.meta.json")
}

fn fetch_text(url: &str) -> Result<String, AppError> {
    let response = ureq::get(url)
        .call()
        .map_err(|err| AppError::Other {
            message: format!("failed to fetch {url}: {err}"),
        })?;
    response
        .into_string()
        .map_err(|err| AppError::Other {
            message: format!("failed to read {url}: {err}"),
        })
}

fn verify_signature(public_key: &[u8], message: &[u8], sig_hex: &str) -> Result<(), AppError> {
    let key = VerifyingKey::from_bytes(public_key.try_into().map_err(|_| AppError::Other {
        message: "invalid public key length".to_string(),
    })?)
    .map_err(|err| AppError::Other {
        message: format!("invalid public key: {err}"),
    })?;
    let sig_bytes = decode_hex(sig_hex)?;
    let signature = Signature::from_slice(sig_bytes.as_slice()).map_err(|err| AppError::Other {
        message: format!("invalid signature: {err}"),
    })?;

    key.verify(message, &signature)
        .map_err(|err| AppError::Other {
            message: format!("manifest signature verification failed: {err}"),
        })
}

fn decode_hex(value: &str) -> Result<Vec<u8>, AppError> {
    let value = value.trim();
    if value.len() % 2 != 0 {
        return Err(AppError::Other {
            message: "invalid hex length".to_string(),
        });
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut chars = value.chars();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        let high = high.to_digit(16).ok_or_else(|| AppError::Other {
            message: "invalid hex digit".to_string(),
        })?;
        let low = low.to_digit(16).ok_or_else(|| AppError::Other {
            message: "invalid hex digit".to_string(),
        })?;
        bytes.push(((high << 4) | low) as u8);
    }
    Ok(bytes)
}

fn is_zero_key(bytes: &[u8]) -> bool {
    bytes.iter().all(|value| *value == 0)
}

fn current_epoch_secs() -> Result<u64, AppError> {
    let now = SystemTime::now();
    let duration = now.duration_since(UNIX_EPOCH).map_err(|err| AppError::Other {
        message: format!("invalid system time: {err}"),
    })?;
    Ok(duration.as_secs())
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
}
