use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;
use tempfile::TempDir;
use xz2::read::XzDecoder;
use zip::ZipArchive;

use crate::cache::{Cache, INSTALL_MARKER_FILE};
use crate::error::AppError;
use crate::manifest::{PackageFormat, ResolvedPackage};

pub fn install(
    cache: &Cache,
    tool: &str,
    version: &str,
    package: &ResolvedPackage,
) -> Result<PathBuf, AppError> {
    cache.with_lock(|| {
        let version_dir = cache.tool_version_dir(tool, version);
        let tool_dir = cache.root().join(tool);
        fs::create_dir_all(&tool_dir)?;
        let tmp_dir = TempDir::new_in(cache.root())?;
        let archive_path = tmp_dir.path().join("archive");
        let staged_parent = TempDir::new_in(&tool_dir)?;
        let staged_version_dir = staged_parent.path().join(version);

        // アーカイブをダウンロード
        let size = download(&package.url, &archive_path, &package.sha256)?;
        if let Some(expected) = package.size {
            if expected != size {
                return Err(AppError::Cache {
                    message: format!("download size mismatch: expected {expected}, got {size}"),
                });
            }
        }

        fs::create_dir_all(&staged_version_dir)?;

        // アーカイブを展開・正規化して最終的なbin_pathsを取得
        let final_bin_paths = match package.format {
            PackageFormat::File => {
                if package.bin_paths.len() > 1 {
                    return Err(AppError::Cache {
                        message: "file package cannot define multiple bin_paths".to_string(),
                    });
                }
                let target = if let Some(path) = package.bin_paths.first() {
                    staged_version_dir.join(path)
                } else {
                    primary_bin_path_for_dir(&staged_version_dir, tool, &package.bin_paths)
                };
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&archive_path, &target)?;
                make_executable(&target)?;
                package.bin_paths.clone()
            }
            PackageFormat::TarGz => {
                unpack_tar_gz(&archive_path, &staged_version_dir)?;
                let bin_paths = normalize_unpacked_layout(&staged_version_dir, &package.bin_paths)?;
                finalize_unpacked_bins(&staged_version_dir, &bin_paths, "tar.gz")?;
                bin_paths
            }
            PackageFormat::TarXz => {
                unpack_tar_xz(&archive_path, &staged_version_dir)?;
                let bin_paths = normalize_unpacked_layout(&staged_version_dir, &package.bin_paths)?;
                finalize_unpacked_bins(&staged_version_dir, &bin_paths, "tar.xz")?;
                bin_paths
            }
            PackageFormat::Zip => {
                unpack_zip(&archive_path, &staged_version_dir)?;
                let bin_paths = normalize_unpacked_layout(&staged_version_dir, &package.bin_paths)?;
                finalize_unpacked_bins(&staged_version_dir, &bin_paths, "zip")?;
                bin_paths
            }
        };

        fs::write(staged_version_dir.join(INSTALL_MARKER_FILE), b"ok")?;
        if version_dir.exists() {
            fs::remove_dir_all(&version_dir)?;
        }
        fs::rename(&staged_version_dir, &version_dir)?;

        let final_install_path = primary_bin_path(cache, tool, version, &final_bin_paths);
        Ok(final_install_path)
    })
}

fn finalize_unpacked_bins(
    unpack_dir: &Path,
    bin_paths: &[String],
    format: &str,
) -> Result<(), AppError> {
    if bin_paths.is_empty() {
        return Err(AppError::Cache {
            message: format!("{format} package missing bin_paths"),
        });
    }

    for bin_path in bin_paths {
        let source = unpack_dir.join(bin_path);
        if !source.exists() {
            return Err(AppError::Cache {
                message: format!("bin_path not found in archive: {}", source.display()),
            });
        }
        make_executable(&source)?;
    }

    Ok(())
}

fn normalize_unpacked_layout(
    unpack_dir: &Path,
    bin_paths: &[String],
) -> Result<Vec<String>, AppError> {
    let mut roots = Vec::new();
    for entry in fs::read_dir(unpack_dir)? {
        roots.push(entry?);
    }
    if roots.len() != 1 {
        return Ok(bin_paths.to_vec());
    }
    let root = &roots[0];
    if !root.path().is_dir() {
        return Ok(bin_paths.to_vec());
    }
    let root_name = root.file_name().to_string_lossy().to_string();
    let prefix = format!("{root_name}/");
    if !bin_paths.iter().all(|path| path.starts_with(&prefix)) {
        return Ok(bin_paths.to_vec());
    }

    let root_path = root.path();
    for entry in fs::read_dir(&root_path)? {
        let entry = entry?;
        let target = unpack_dir.join(entry.file_name());
        fs::rename(entry.path(), target)?;
    }
    fs::remove_dir(root_path)?;

    Ok(bin_paths
        .iter()
        .map(|path| path.trim_start_matches(&prefix).to_string())
        .collect())
}

fn primary_bin_path(cache: &Cache, tool: &str, version: &str, bin_paths: &[String]) -> PathBuf {
    let version_dir = cache.tool_version_dir(tool, version);
    primary_bin_path_for_dir(&version_dir, tool, bin_paths)
}

fn primary_bin_path_for_dir(version_dir: &Path, tool: &str, bin_paths: &[String]) -> PathBuf {
    if let Some(bin_path) = find_bin_path(tool, bin_paths) {
        return version_dir.join(bin_path);
    }
    if let Some(first) = bin_paths.first() {
        return version_dir.join(first);
    }
    let mut name = tool.to_string();
    if cfg!(windows) {
        name.push_str(".exe");
    }
    version_dir.join(name)
}

fn find_bin_path<'a>(bin_name: &str, bin_paths: &'a [String]) -> Option<&'a str> {
    bin_paths.iter().find_map(|path| {
        let stem = Path::new(path).file_stem()?.to_str()?;
        if stem == bin_name {
            Some(path.as_str())
        } else {
            None
        }
    })
}

fn download(url: &str, dest: &Path, expected_sha256: &str) -> Result<u64, AppError> {
    let response = ureq::get(url).call().map_err(|err| AppError::Cache {
        message: format!("download failed for {url}: {err}"),
    })?;

    let mut reader = response.into_reader();
    let mut file = File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;

    loop {
        let count = reader.read(&mut buf)?;
        if count == 0 {
            break;
        }
        hasher.update(&buf[..count]);
        file.write_all(&buf[..count])?;
        size += count as u64;
    }

    let actual = format!("{:x}", hasher.finalize());
    if normalize_hex(expected_sha256) != normalize_hex(&actual) {
        return Err(AppError::Cache {
            message: "sha256 mismatch for download".to_string(),
        });
    }

    Ok(size)
}

fn unpack_tar_gz(archive_path: &Path, target: &Path) -> Result<(), AppError> {
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive.unpack(target)?;
    Ok(())
}

fn unpack_tar_xz(archive_path: &Path, target: &Path) -> Result<(), AppError> {
    let file = File::open(archive_path)?;
    let decoder = XzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive.unpack(target)?;
    Ok(())
}

fn unpack_zip(archive_path: &Path, target: &Path) -> Result<(), AppError> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(|err| AppError::Cache {
        message: format!("failed to read zip: {err}"),
    })?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|err| AppError::Cache {
            message: format!("failed to read zip entry: {err}"),
        })?;
        let path = match entry.enclosed_name() {
            Some(path) => target.join(path),
            None => continue,
        };
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&path)?;
        std::io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

fn normalize_hex(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn make_executable(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    use crate::manifest::ResolvedPackage;

    #[test]
    fn normalizes_hex() {
        assert_eq!(normalize_hex(" AbCd "), "abcd");
    }

    #[test]
    fn bin_name_and_path_selection_work() {
        let paths = vec!["bin/npm".to_string(), "bin/node".to_string()];
        assert_eq!(find_bin_path("node", &paths), Some("bin/node"));
        assert_eq!(find_bin_path("bun", &paths), None);

        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        let picked = primary_bin_path(&cache, "node", "22", &paths);
        assert!(picked.ends_with("node/22/bin/node"));
        let fallback = primary_bin_path(
            &cache,
            "node",
            "22",
            &[String::from("nested/npm"), String::from("nested/npx")],
        );
        assert!(fallback.ends_with("node/22/nested/npm"));
    }

    #[test]
    fn unpack_helpers_and_make_executable_error_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("missing");
        let target = temp.path().join("out");
        assert!(unpack_tar_gz(&missing, &target).is_err());
        assert!(unpack_tar_xz(&missing, &target).is_err());
        assert!(unpack_zip(&missing, &target).is_err());
        assert!(make_executable(&missing).is_err());
    }

    #[test]
    fn finalize_unpacked_bins_validates_missing_bins() {
        let temp = tempfile::tempdir().expect("tempdir");
        let unpack = temp.path().join("unpack");
        fs::create_dir_all(&unpack).expect("mkdir");

        let err = finalize_unpacked_bins(&unpack, &[], "tar.gz").expect_err("missing paths");
        assert!(matches!(err, AppError::Cache { .. }));

        let err = finalize_unpacked_bins(&unpack, &[String::from("bin/node")], "tar.gz")
            .expect_err("missing source");
        assert!(matches!(err, AppError::Cache { .. }));
    }

    #[test]
    fn finalize_unpacked_bins_accepts_existing_bins() {
        let temp = tempfile::tempdir().expect("tempdir");
        let unpack = temp.path().join("unpack");
        let bin = unpack.join("bin");
        fs::create_dir_all(&bin).expect("mkdir");
        let node_path = bin.join("node");
        fs::write(&node_path, b"node-binary").expect("write");

        finalize_unpacked_bins(&unpack, &[String::from("bin/node")], "tar.gz")
            .expect("finalize unpack");
    }

    #[test]
    fn normalize_unpacked_layout_flattens_single_root_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let unpack = temp.path().join("unpack");
        let root = unpack.join("node-v24.3.1-darwin-arm64");
        let source_bin = root.join("bin");
        fs::create_dir_all(&source_bin).expect("mkdir");
        fs::write(source_bin.join("node"), b"bin").expect("write");

        let normalized = normalize_unpacked_layout(
            &unpack,
            &[String::from("node-v24.3.1-darwin-arm64/bin/node")],
        )
        .expect("normalize");

        assert_eq!(normalized, vec![String::from("bin/node")]);
        assert!(unpack.join("bin/node").exists());
        assert!(!root.exists());
    }

    #[test]
    fn install_file_package_from_local_http_server() {
        let payload = b"#!/bin/sh\necho ok\n".to_vec();
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let sha256 = format!("{:x}", hasher.finalize());

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let payload_for_thread = payload.clone();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                payload_for_thread.len()
            );
            std::io::Write::write_all(&mut stream, response.as_bytes()).expect("header");
            std::io::Write::write_all(&mut stream, &payload_for_thread).expect("body");
        });

        let temp = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(temp.path().to_path_buf());
        let package = ResolvedPackage {
            url: format!("http://{addr}/tool"),
            sha256,
            size: Some(payload.len() as u64),
            format: PackageFormat::File,
            bin_paths: vec![],
        };

        let bin = install(&cache, "toolx", "1.0.0", &package).expect("install");
        assert!(bin.exists());
        assert!(cache.is_installed("toolx", "1.0.0"));
        handle.join().expect("server join");
    }
}
