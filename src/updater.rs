use crate::error::AppError;
use crate::manifest::Target;
use semver::Version;
use serde::Deserialize;
use serde_json;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufRead, Read, Write};
use std::path::Path;

#[derive(Debug, Deserialize)]
pub(crate) struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Asset {
    name: String,
    browser_download_url: String,
}

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn fetch_release_from(version: Option<&str>, base_url: &str) -> Result<Release, AppError> {
    let url = match version {
        None => format!("{base_url}/repos/tiibun/ampland/releases/latest"),
        Some(v) => {
            let v = v.trim_start_matches('v');
            format!("{base_url}/repos/tiibun/ampland/releases/tags/v{v}")
        }
    };

    let response = match ureq::get(&url)
        .set("User-Agent", &format!("ampland/{CURRENT_VERSION}"))
        .call()
    {
        Ok(r) => r,
        Err(ureq::Error::Status(403 | 429, _)) => {
            return Err(AppError::Other {
                message: "GitHub API rate limit exceeded; try again later".to_string(),
            });
        }
        Err(ureq::Error::Status(404, _)) => {
            return Err(AppError::Other {
                message: format!(
                    "release v{} not found on GitHub",
                    version.map(|v| v.trim_start_matches('v')).unwrap_or("latest")
                ),
            });
        }
        Err(err) => {
            return Err(AppError::Other {
                message: format!("failed to fetch release info: {err}"),
            });
        }
    };

    let body = response
        .into_string()
        .map_err(|err| AppError::Other {
            message: format!("failed to read release response: {err}"),
        })?;

    let release: Release = serde_json::from_str(&body).map_err(|err| AppError::Other {
        message: format!("failed to parse release info: {err}"),
    })?;

    Ok(release)
}

#[allow(dead_code)]
fn fetch_release(version: Option<&str>) -> Result<Release, AppError> {
    fetch_release_from(version, "https://api.github.com")
}

fn asset_name_for_target(platform: &str, arch: &str) -> Result<String, AppError> {
    match (platform, arch) {
        ("macos", "arm64") => Ok("ampland-macos-arm64".to_string()),
        ("macos", "x64") => Ok("ampland-macos-x64".to_string()),
        ("linux", "x64") => Ok("ampland-linux-x64".to_string()),
        ("windows", "x64") => Ok("ampland-windows-x64.exe".to_string()),
        (p, a) => Err(AppError::Other {
            message: format!("no release asset for platform={p} arch={a}"),
        }),
    }
}

fn asset_name_for_current_target() -> Result<String, AppError> {
    let t = Target::current()?;
    asset_name_for_target(&t.platform, &t.arch)
}

fn download_with_hash(url: &str, dest: &Path) -> Result<String, AppError> {
    let response = ureq::get(url)
        .set("User-Agent", &format!("ampland/{CURRENT_VERSION}"))
        .call()
        .map_err(|err| AppError::Other {
            message: format!("download failed for {url}: {err}"),
        })?;

    let mut reader = response.into_reader();
    let mut file = File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];

    loop {
        let count = reader.read(&mut buf)?;
        if count == 0 {
            break;
        }
        hasher.update(&buf[..count]);
        file.write_all(&buf[..count])?;
    }

    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg_attr(not(windows), allow(unused_variables))]
fn replace_binary(temp_path: &Path, target: &Path, new_ver: &str) -> Result<(), AppError> {
    std::fs::rename(temp_path, target).map_err(|err| {
        #[cfg(windows)]
        if err.raw_os_error() == Some(32) {
            // ERROR_SHARING_VIOLATION: binary is locked (running)
            return AppError::Other {
                message: format!(
                    "cannot replace running binary on Windows; download the new version manually: \
                     https://github.com/tiibun/ampland/releases/tag/v{new_ver}"
                ),
            };
        }
        AppError::Other {
            message: format!("failed to replace binary: {err}"),
        }
    })
}

fn fetch_text(url: &str) -> Result<String, AppError> {
    let response = ureq::get(url)
        .set("User-Agent", &format!("ampland/{CURRENT_VERSION}"))
        .call()
        .map_err(|err| AppError::Other {
            message: format!("failed to fetch {url}: {err}"),
        })?;
    response.into_string().map_err(|err| AppError::Other {
        message: format!("failed to read response from {url}: {err}"),
    })
}

fn self_update_inner(
    version: Option<&str>,
    yes: bool,
    api_base: &str,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<(), AppError> {
    let current = CURRENT_VERSION;
    let release = fetch_release_from(version, api_base)?;
    let new_ver = release.tag_name.trim_start_matches('v');

    if new_ver == current {
        if version.is_none() {
            writeln!(out, "already up to date ({current})").ok();
        } else {
            writeln!(out, "already at version {current}").ok();
        }
        return Ok(());
    }

    let current_semver = Version::parse(current).ok();
    let new_semver = Version::parse(new_ver).ok();
    let is_downgrade = match (current_semver, new_semver) {
        (Some(c), Some(n)) => n < c,
        _ => false,
    };

    if !yes {
        let action = if is_downgrade { "downgrade" } else { "update" };
        write!(out, "{action} {current} -> {new_ver}? [y/N] ").ok();
        let _ = out.flush();
        let mut line = String::new();
        input.read_line(&mut line).ok();
        let trimmed = line.trim();
        if trimmed != "y" && trimmed != "Y" {
            return Ok(());
        }
    }

    let asset_name = asset_name_for_current_target()?;
    let sha256_asset_name = format!("{asset_name}.sha256");

    let binary_asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| AppError::Other {
            message: format!("no release asset found for {asset_name} in release v{new_ver}"),
        })?;

    let sha256_asset = release
        .assets
        .iter()
        .find(|a| a.name == sha256_asset_name)
        .ok_or_else(|| AppError::Other {
            message: format!(
                "no checksum file found for {asset_name} in release v{new_ver}"
            ),
        })?;

    // Take only the first whitespace-separated token to handle both bare hex format
    // and `sha256sum`-style "hex  filename" format defensively.
    let sidecar_content = fetch_text(&sha256_asset.browser_download_url)?;
    let expected_hash = sidecar_content
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    let current_exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|err| AppError::Other {
            message: format!("cannot determine current executable path: {err}"),
        })?;

    let exe_dir = current_exe.parent().ok_or_else(|| AppError::Other {
        message: "cannot determine executable directory".to_string(),
    })?;

    let tmp_path = exe_dir.join(format!("ampland-update-{new_ver}.tmp"));

    let actual_hash = download_with_hash(&binary_asset.browser_download_url, &tmp_path)
        .map_err(|err| {
            let _ = std::fs::remove_file(&tmp_path);
            if let AppError::Io { ref message } = err {
                if message.contains("permission denied") || message.contains("Permission denied") {
                    return AppError::Other {
                        message: format!(
                            "cannot write to {}: permission denied (try sudo)",
                            exe_dir.display()
                        ),
                    };
                }
            }
            err
        })?;

    if actual_hash != expected_hash {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::Other {
            message: "checksum mismatch: download may be corrupted".to_string(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&tmp_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&tmp_path, perms)?;
    }

    replace_binary(&tmp_path, &current_exe, new_ver).map_err(|err| {
        let _ = std::fs::remove_file(&tmp_path);
        err
    })?;

    writeln!(out, "updated to {new_ver}").ok();
    Ok(())
}

/// Update ampland, writing output to stdout and reading confirmation from stdin.
/// Pass `quiet: true` to suppress all output (for `--quiet` mode).
pub fn self_update(version: Option<&str>, yes: bool, quiet: bool) -> Result<(), AppError> {
    if quiet {
        self_update_inner(
            version,
            yes,
            "https://api.github.com",
            &mut std::io::sink(),
            &mut std::io::BufReader::new(std::io::stdin()),
        )
    } else {
        self_update_inner(
            version,
            yes,
            "https://api.github.com",
            &mut std::io::stdout(),
            &mut std::io::BufReader::new(std::io::stdin()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn serve_once(body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            std::io::Write::write_all(&mut stream, resp.as_bytes()).expect("write");
        });
        format!("http://{addr}")
    }

    #[test]
    fn fetch_release_parses_assets() {
        let body = r#"{
            "tag_name": "v0.3.0",
            "assets": [
                {"name": "ampland-macos-arm64", "browser_download_url": "http://example.com/ampland-macos-arm64"},
                {"name": "ampland-macos-arm64.sha256", "browser_download_url": "http://example.com/ampland-macos-arm64.sha256"}
            ]
        }"#.to_string();
        let base_url = serve_once(body);
        let release = fetch_release_from(None, &base_url).expect("fetch");
        assert_eq!(release.tag_name, "v0.3.0");
        assert_eq!(release.assets.len(), 2);
        assert_eq!(release.assets[0].name, "ampland-macos-arm64");
    }

    #[test]
    fn fetch_release_with_version_builds_correct_url() {
        let body = r#"{"tag_name": "v0.2.7", "assets": []}"#.to_string();
        let base_url = serve_once(body);
        let release = fetch_release_from(Some("0.2.7"), &base_url).expect("fetch");
        assert_eq!(release.tag_name, "v0.2.7");
    }

    #[test]
    fn asset_name_macos_arm64() {
        assert_eq!(
            asset_name_for_target("macos", "arm64").expect("ok"),
            "ampland-macos-arm64"
        );
    }

    #[test]
    fn asset_name_macos_x64() {
        assert_eq!(
            asset_name_for_target("macos", "x64").expect("ok"),
            "ampland-macos-x64"
        );
    }

    #[test]
    fn asset_name_linux_x64() {
        assert_eq!(
            asset_name_for_target("linux", "x64").expect("ok"),
            "ampland-linux-x64"
        );
    }

    #[test]
    fn asset_name_windows_x64() {
        assert_eq!(
            asset_name_for_target("windows", "x64").expect("ok"),
            "ampland-windows-x64.exe"
        );
    }

    #[test]
    fn asset_name_unknown_platform_errors() {
        assert!(asset_name_for_target("freebsd", "x64").is_err());
    }

    use sha2::{Digest, Sha256};

    fn serve_bytes_once(payload: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                payload.len()
            );
            std::io::Write::write_all(&mut stream, resp.as_bytes()).expect("header");
            std::io::Write::write_all(&mut stream, &payload).expect("body");
        });
        format!("http://{addr}/binary")
    }

    #[test]
    fn download_with_hash_returns_correct_digest() {
        let payload = b"fake-binary-content".to_vec();
        let expected_hex = {
            let mut h = Sha256::new();
            h.update(&payload);
            format!("{:x}", h.finalize())
        };
        let url = serve_bytes_once(payload);
        let temp = tempfile::tempdir().expect("tempdir");
        let dest = temp.path().join("binary");
        let actual_hex = download_with_hash(&url, &dest).expect("download");
        assert_eq!(actual_hex, expected_hex);
        assert!(dest.exists());
    }

    #[test]
    fn replace_binary_swaps_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("ampland");
        std::fs::write(&target, b"old").expect("write old");

        let new_bin = temp.path().join("ampland.new.tmp");
        std::fs::write(&new_bin, b"new").expect("write new");

        replace_binary(&new_bin, &target, "0.3.0").expect("replace");
        assert_eq!(std::fs::read(&target).expect("read"), b"new");
        assert!(!new_bin.exists());
    }

    #[test]
    fn replace_binary_errors_on_missing_temp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("ampland");
        let missing = temp.path().join("missing.tmp");
        assert!(replace_binary(&missing, &target, "0.3.0").is_err());
    }

    #[test]
    fn self_update_already_up_to_date_when_same_version() {
        let current = CURRENT_VERSION;
        let body = format!(r#"{{"tag_name":"v{current}","assets":[]}}"#);
        let base_url = serve_once(body);
        let mut out = Vec::<u8>::new();
        let mut inp = std::io::Cursor::new(b"" as &[u8]);
        let result = self_update_inner(Some(current), true, &base_url, &mut out, &mut inp);
        assert!(result.is_ok());
        // When a specific version is requested and matches, prints "already at version"
        assert!(String::from_utf8(out).unwrap().contains("already at version"));
    }

    #[test]
    fn self_update_already_up_to_date_latest() {
        let current = CURRENT_VERSION;
        let body = format!(r#"{{"tag_name":"v{current}","assets":[]}}"#);
        let base_url = serve_once(body);
        let mut out = Vec::<u8>::new();
        let mut inp = std::io::Cursor::new(b"" as &[u8]);
        // version: None → fetching latest, which matches current
        let result = self_update_inner(None, true, &base_url, &mut out, &mut inp);
        assert!(result.is_ok());
        // When fetching latest and already current, prints "already up to date"
        assert!(String::from_utf8(out).unwrap().contains("already up to date"));
    }

    fn serve_status_once(status_line: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let resp = format!("{status_line}\r\nContent-Length: 0\r\n\r\n");
            std::io::Write::write_all(&mut stream, resp.as_bytes()).expect("write");
        });
        format!("http://{addr}")
    }

    #[test]
    fn fetch_release_rate_limit_returns_error() {
        let base_url = serve_status_once("HTTP/1.1 403 Forbidden");
        let err = fetch_release_from(None, &base_url).expect_err("should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("rate limit"),
            "expected 'rate limit' in error, got: {msg}"
        );
    }

    #[test]
    fn self_update_checksum_mismatch_returns_error() {
        let binary_payload = b"fake-binary-content".to_vec();

        // Serve a sha256 sidecar with a wrong (all-zeros) hash
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        // Server 1: binary download
        let binary_url = serve_bytes_once(binary_payload);

        // Server 2: sha256 sidecar (wrong hash) — serve_once returns http://addr,
        // which is used directly as browser_download_url in the release JSON.
        let sha256_body = format!("{wrong_hash}  ampland-macos-arm64\n");
        let sha256_base = serve_once(sha256_body);
        // The sha256 URL must end with the asset name; append a path component.
        let sha256_url = format!("{sha256_base}/ampland-macos-arm64.sha256");

        // Server 3: release JSON referencing the above URLs
        let release_body = format!(
            r#"{{"tag_name":"v99.9.9","assets":[
                {{"name":"ampland-macos-arm64","browser_download_url":"{binary_url}"}},
                {{"name":"ampland-macos-arm64.sha256","browser_download_url":"{sha256_url}"}}
            ]}}"#
        );
        let base_url = serve_once(release_body);

        let mut out = Vec::<u8>::new();
        let mut inp = std::io::Cursor::new(b"" as &[u8]);
        let result = self_update_inner(Some("99.9.9"), true, &base_url, &mut out, &mut inp);
        let err = result.expect_err("should fail with checksum mismatch");
        let msg = err.to_string();
        assert!(
            msg.contains("checksum mismatch"),
            "expected 'checksum mismatch' in error, got: {msg}"
        );
    }

    #[test]
    fn self_update_declined_by_user_returns_ok() {
        // User sees prompt for v99.0.0 and types "n" — no download should happen.
        // Only the release JSON server is needed; no binary or sha256 servers.
        let release_body = format!(
            r#"{{"tag_name":"v99.0.0","assets":[
                {{"name":"ampland-macos-arm64","browser_download_url":"http://127.0.0.1:1/never"}},
                {{"name":"ampland-macos-arm64.sha256","browser_download_url":"http://127.0.0.1:1/never.sha256"}}
            ]}}"#
        );
        let base_url = serve_once(release_body);
        let mut out = Vec::<u8>::new();
        // User types "n"
        let mut inp = std::io::Cursor::new(b"n\n" as &[u8]);
        let result = self_update_inner(None, false, &base_url, &mut out, &mut inp);
        assert!(result.is_ok());
        // Confirm the prompt was shown
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("update") || output.contains("downgrade"));
    }
}
