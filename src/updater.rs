use crate::error::AppError;
use crate::manifest::Target;
use serde::Deserialize;
use serde_json;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
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
pub fn fetch_release(version: Option<&str>) -> Result<Release, AppError> {
    fetch_release_from(version, "https://api.github.com")
}

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn asset_name_for_current_target() -> Result<String, AppError> {
    let t = Target::current()?;
    asset_name_for_target(&t.platform, &t.arch)
}

#[allow(dead_code)]
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

#[allow(dead_code)]
#[allow(unused_variables)]
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
}
