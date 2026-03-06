use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const USER_AGENT: &str = "ampland-manifest-generate";
pub(crate) const OUTPUT_DIR_DEFAULT: &str = "assets/manifest";
pub(crate) const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolSelector {
    Node,
    Python,
}

impl ToolSelector {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "node" => Some(Self::Node),
            "python" => Some(Self::Python),
            _ => None,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GeneratorArgs {
    pub(crate) output_dir: PathBuf,
    pub(crate) tool: Option<ToolSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolManifest {
    pub(crate) version: u32,
    pub(crate) generated_at: String,
    #[serde(rename = "tool")]
    pub(crate) tools: Vec<ToolEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolEntry {
    pub(crate) name: String,
    pub(crate) vendor: String,
    pub(crate) default_version: String,
    #[serde(rename = "version")]
    pub(crate) versions: Vec<ToolVersion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManifestWriteOutcome {
    Created,
    Updated,
    Unchanged,
}

impl ManifestWriteOutcome {
    pub(crate) fn summary(self, path: &Path) -> String {
        match self {
            Self::Created | Self::Updated => format!("updated {}", path.display()),
            Self::Unchanged => format!("unchanged {}", path.display()),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
}

pub(crate) fn parse_args() -> Result<GeneratorArgs, String> {
    parse_args_from(env::args().skip(1))
}

fn parse_args_from<I>(args: I) -> Result<GeneratorArgs, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let mut output_dir = None;
    let mut positional = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--output-dir" {
            let value = args
                .next()
                .ok_or_else(|| "missing value for --output-dir".to_string())?;
            output_dir = Some(PathBuf::from(value));
        } else if arg.starts_with("--") {
            return Err(format!("unknown argument: {arg}"));
        } else {
            positional.push(arg);
        }
    }

    let tool = match positional.as_slice() {
        [] => None,
        [value] => {
            if let Some(tool) = ToolSelector::parse(value) {
                Some(tool)
            } else if output_dir.is_some() {
                return Err(unknown_tool_selector(value));
            } else {
                output_dir = Some(PathBuf::from(value));
                None
            }
        }
        [dir, value] => {
            if output_dir.is_some() {
                return Err(format!("unexpected positional output directory: {dir}"));
            }
            output_dir = Some(PathBuf::from(dir));
            Some(ToolSelector::parse(value).ok_or_else(|| unknown_tool_selector(value))?)
        }
        _ => {
            return Err(
                "too many positional arguments (expected [output-dir] [node|python])".to_string(),
            )
        }
    };

    Ok(GeneratorArgs {
        output_dir: output_dir.unwrap_or_else(|| PathBuf::from(OUTPUT_DIR_DEFAULT)),
        tool,
    })
}

fn unknown_tool_selector(value: &str) -> String {
    format!("unknown tool selector: {value} (expected one of: node, python)")
}

pub(crate) fn selected_tools_label(tool: Option<ToolSelector>) -> &'static str {
    match tool {
        Some(tool) => tool.name(),
        None => "node and python",
    }
}

pub(crate) fn write_manifest(
    path: &Path,
    manifest: &ToolManifest,
) -> Result<ManifestWriteOutcome, String> {
    let output = toml::to_string_pretty(manifest).map_err(|err| err.to_string())?;
    let existed = path.exists();

    if existed {
        let existing_text = fs::read_to_string(path).map_err(|err| err.to_string())?;
        let existing_manifest: ToolManifest =
            toml::from_str(&existing_text).map_err(|err| err.to_string())?;
        if manifests_match_ignoring_generated_at(&existing_manifest, manifest) {
            return Ok(ManifestWriteOutcome::Unchanged);
        }
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    fs::write(path, output).map_err(|err| err.to_string())?;
    if existed {
        Ok(ManifestWriteOutcome::Updated)
    } else {
        Ok(ManifestWriteOutcome::Created)
    }
}

fn manifests_match_ignoring_generated_at(
    existing: &ToolManifest,
    generated: &ToolManifest,
) -> bool {
    existing.version == generated.version && existing.tools == generated.tools
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use std::path::PathBuf;

    use super::{
        parse_args_from, selected_tools_label, write_manifest, GeneratorArgs, ManifestWriteOutcome,
        ToolEntry, ToolManifest, ToolSelector, ToolVersion, OUTPUT_DIR_DEFAULT,
    };

    fn sample_manifest(generated_at: &str, default_version: &str) -> ToolManifest {
        ToolManifest {
            version: 1,
            generated_at: generated_at.to_string(),
            tools: vec![ToolEntry {
                name: "node".to_string(),
                vendor: "nodejs".to_string(),
                default_version: default_version.to_string(),
                versions: vec![ToolVersion {
                    ver: default_version.to_string(),
                    platform: "macos".to_string(),
                    arch: "arm64".to_string(),
                    url: format!("https://example.com/node-{default_version}.tar.gz"),
                    sha256: format!("hash-{default_version}"),
                    format: "tar.gz".to_string(),
                    bin_paths: vec!["bin/node".to_string()],
                }],
            }],
        }
    }

    fn parse(values: &[&str]) -> Result<GeneratorArgs, String> {
        parse_args_from(values.iter().map(|value| value.to_string()))
    }

    #[test]
    fn parses_default_args() {
        assert_eq!(
            parse(&[]).unwrap(),
            GeneratorArgs {
                output_dir: PathBuf::from(OUTPUT_DIR_DEFAULT),
                tool: None,
            }
        );
    }

    #[test]
    fn parses_tool_selector_only() {
        assert_eq!(
            parse(&["node"]).unwrap(),
            GeneratorArgs {
                output_dir: PathBuf::from(OUTPUT_DIR_DEFAULT),
                tool: Some(ToolSelector::Node),
            }
        );
    }

    #[test]
    fn parses_positional_output_dir_and_tool() {
        assert_eq!(
            parse(&["path/to/out", "python"]).unwrap(),
            GeneratorArgs {
                output_dir: PathBuf::from("path/to/out"),
                tool: Some(ToolSelector::Python),
            }
        );
    }

    #[test]
    fn parses_flag_output_dir_and_tool() {
        assert_eq!(
            parse(&["--output-dir", "path/to/out", "node"]).unwrap(),
            GeneratorArgs {
                output_dir: PathBuf::from("path/to/out"),
                tool: Some(ToolSelector::Node),
            }
        );
    }

    #[test]
    fn rejects_unknown_tool_selector() {
        assert_eq!(
            parse(&["--output-dir", "path/to/out", "ruby"]).unwrap_err(),
            "unknown tool selector: ruby (expected one of: node, python)"
        );
    }

    #[test]
    fn rejects_extra_positional_arguments() {
        assert_eq!(
            parse(&["path/to/out", "node", "extra"]).unwrap_err(),
            "too many positional arguments (expected [output-dir] [node|python])"
        );
    }

    #[test]
    fn formats_selected_tools_label() {
        assert_eq!(selected_tools_label(None), "node and python");
        assert_eq!(selected_tools_label(Some(ToolSelector::Node)), "node");
        assert_eq!(selected_tools_label(Some(ToolSelector::Python)), "python");
    }

    #[test]
    fn write_manifest_creates_new_file() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("node.toml");

        let outcome = write_manifest(&path, &sample_manifest("2026-03-06T00:00:00Z", "24.0.0"))
            .expect("write manifest");

        assert_eq!(outcome, ManifestWriteOutcome::Created);
        assert!(path.exists());
    }

    #[test]
    fn write_manifest_skips_rewrite_when_only_timestamp_changes() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("node.toml");

        write_manifest(&path, &sample_manifest("2026-03-06T00:00:00Z", "24.0.0"))
            .expect("initial write");
        let outcome = write_manifest(&path, &sample_manifest("2026-03-07T00:00:00Z", "24.0.0"))
            .expect("second write");
        let saved: ToolManifest =
            toml::from_str(&std::fs::read_to_string(&path).expect("read manifest"))
                .expect("parse manifest");

        assert_eq!(outcome, ManifestWriteOutcome::Unchanged);
        assert_eq!(saved.generated_at, "2026-03-06T00:00:00Z");
    }

    #[test]
    fn write_manifest_updates_when_manifest_changes() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("node.toml");

        write_manifest(&path, &sample_manifest("2026-03-06T00:00:00Z", "24.0.0"))
            .expect("initial write");
        let outcome = write_manifest(&path, &sample_manifest("2026-03-07T00:00:00Z", "25.0.0"))
            .expect("updated write");
        let saved: ToolManifest =
            toml::from_str(&std::fs::read_to_string(&path).expect("read manifest"))
                .expect("parse manifest");

        assert_eq!(outcome, ManifestWriteOutcome::Updated);
        assert_eq!(saved.generated_at, "2026-03-07T00:00:00Z");
        assert_eq!(saved.tools[0].default_version, "25.0.0");
    }
}
