use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub(crate) const USER_AGENT: &str = "ampland-manifest-generate";
pub(crate) const OUTPUT_DIR_DEFAULT: &str = "assets/manifest";
pub(crate) const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub(crate) struct ToolManifest {
    pub(crate) version: u32,
    pub(crate) generated_at: String,
    #[serde(rename = "tool")]
    pub(crate) tools: Vec<ToolEntry>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolEntry {
    pub(crate) name: String,
    pub(crate) vendor: String,
    pub(crate) default_version: String,
    #[serde(rename = "version")]
    pub(crate) versions: Vec<ToolVersion>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolVersion {
    pub(crate) ver: String,
    pub(crate) platform: String,
    pub(crate) arch: String,
    pub(crate) url: String,
    pub(crate) sha256: String,
    pub(crate) format: String,
    pub(crate) bin_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TargetSpec {
    pub(crate) platform: &'static str,
    pub(crate) arch: &'static str,
}

pub(crate) fn parse_args() -> Result<PathBuf, String> {
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

pub(crate) fn write_manifest(path: &Path, manifest: &ToolManifest) -> Result<(), String> {
    let output = toml::to_string_pretty(manifest).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    fs::write(path, output).map_err(|err| err.to_string())?;
    Ok(())
}

pub(crate) fn default_targets() -> Vec<TargetSpec> {
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

pub(crate) fn fetch_text(url: &str) -> Result<String, String> {
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|err| err.to_string())?;
    read_response_text(response, url)
}

fn read_response_text(response: ureq::Response, url: &str) -> Result<String, String> {
    let mut reader = response.into_reader();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut total = 0usize;

    loop {
        let read = reader.read(&mut chunk).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read);
        if total > MAX_TEXT_BYTES {
            return Err(format!(
                "response too big for {url} (>{} bytes)",
                MAX_TEXT_BYTES
            ));
        }
        buf.extend_from_slice(&chunk[..read]);
    }

    String::from_utf8(buf).map_err(|err| err.to_string())
}

pub(crate) fn fetch_sha256(url: &str) -> Result<String, String> {
    let text = fetch_text(url)?;
    let hash = text
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("empty sha256 response from {url}"))?;
    Ok(hash.to_string())
}

pub(crate) fn download_and_hash(url: &str) -> Result<String, String> {
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