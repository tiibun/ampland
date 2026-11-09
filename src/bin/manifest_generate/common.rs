use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub(crate) const USER_AGENT: &str = "ampland-manifest-generate";
pub(crate) const OUTPUT_DIR_DEFAULT: &str = "assets/manifest";
pub(crate) const MAX_TEXT_BYTES: usize = 20 * 1024 * 1024;
const FETCH_MAX_ATTEMPTS: u32 = 4;
const FETCH_RETRY_BASE_DELAY: Duration = Duration::from_secs(2);

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

/// Outcome of a single fetch attempt, distinguishing failures worth retrying.
enum FetchError {
    Transient(String),
    Fatal(String),
}

pub(crate) fn fetch_text(url: &str) -> Result<String, String> {
    retry_with_backoff(
        url,
        FETCH_MAX_ATTEMPTS,
        FETCH_RETRY_BASE_DELAY,
        thread::sleep,
        || fetch_text_once(url),
    )
}

fn fetch_text_once(url: &str) -> Result<String, FetchError> {
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(classify_ureq_error)?;
    // A body that fails mid-stream is a network hiccup; size/encoding errors are not.
    read_response_text(response, url).map_err(FetchError::Fatal)
}

fn classify_ureq_error(err: ureq::Error) -> FetchError {
    match &err {
        ureq::Error::Status(code, _) if is_retryable_status(*code) => {
            FetchError::Transient(err.to_string())
        }
        ureq::Error::Status(..) => FetchError::Fatal(err.to_string()),
        ureq::Error::Transport(_) => FetchError::Transient(err.to_string()),
    }
}

fn is_retryable_status(code: u16) -> bool {
    code == 429 || (500..=599).contains(&code)
}

/// Runs `attempt` up to `max_attempts` times, sleeping with exponential backoff
/// between transient failures. Fatal failures are returned immediately.
fn retry_with_backoff<T>(
    url: &str,
    max_attempts: u32,
    base_delay: Duration,
    mut sleep: impl FnMut(Duration),
    mut attempt: impl FnMut() -> Result<T, FetchError>,
) -> Result<T, String> {
    let mut tries = 1;
    loop {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(FetchError::Fatal(message)) => return Err(message),
            Err(FetchError::Transient(message)) if tries >= max_attempts => {
                return Err(format!("{message} (gave up after {tries} attempts)"));
            }
            Err(FetchError::Transient(message)) => {
                let delay = base_delay * 2u32.pow(tries - 1);
                eprintln!(
                    "manifest-generate: {url}: {message}; retrying in {}s (attempt {}/{max_attempts})",
                    delay.as_secs(),
                    tries + 1
                );
                sleep(delay);
                tries += 1;
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use std::path::PathBuf;

    use std::time::Duration;

    use super::{
        is_retryable_status, parse_args_from, retry_with_backoff, selected_tools_label,
        write_manifest, FetchError, GeneratorArgs, ManifestWriteOutcome, ToolEntry, ToolManifest,
        ToolSelector, ToolVersion, OUTPUT_DIR_DEFAULT,
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
    fn retryable_statuses_are_server_errors_and_rate_limits() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(504));
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(403));
    }

    #[test]
    fn retry_recovers_after_transient_failures() {
        let mut sleeps = Vec::new();
        let mut calls = 0;
        let result = retry_with_backoff(
            "https://example.com",
            4,
            Duration::from_secs(2),
            |delay| sleeps.push(delay),
            || {
                calls += 1;
                if calls < 3 {
                    Err(FetchError::Transient("status code 504".to_string()))
                } else {
                    Ok("body")
                }
            },
        );

        assert_eq!(result, Ok("body"));
        assert_eq!(calls, 3);
        assert_eq!(sleeps, vec![Duration::from_secs(2), Duration::from_secs(4)]);
    }

    #[test]
    fn retry_gives_up_after_max_attempts() {
        let mut calls = 0;
        let result: Result<(), String> = retry_with_backoff(
            "https://example.com",
            3,
            Duration::from_secs(1),
            |_| {},
            || {
                calls += 1;
                Err(FetchError::Transient("status code 504".to_string()))
            },
        );

        assert_eq!(
            result,
            Err("status code 504 (gave up after 3 attempts)".to_string())
        );
        assert_eq!(calls, 3);
    }

    #[test]
    fn retry_does_not_repeat_fatal_errors() {
        let mut calls = 0;
        let result: Result<(), String> = retry_with_backoff(
            "https://example.com",
            4,
            Duration::from_secs(1),
            |_| panic!("should not sleep"),
            || {
                calls += 1;
                Err(FetchError::Fatal("status code 404".to_string()))
            },
        );

        assert_eq!(result, Err("status code 404".to_string()));
        assert_eq!(calls, 1);
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
