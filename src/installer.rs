use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;
use tempfile::TempDir;
use xz2::read::XzDecoder;
use zip::ZipArchive;

use crate::cache::Cache;
use crate::error::AppError;
use crate::manifest::{PackageFormat, ResolvedPackage};

pub fn install(
    cache: &Cache,
    tool: &str,
    version: &str,
    package: &ResolvedPackage,
) -> Result<PathBuf, AppError> {
    cache.with_lock(|| {
        let bin_dir = cache.tool_bin_dir(tool, version);
        let install_path = primary_bin_path(cache, tool, version, &package.bin_paths);
        if install_path.exists() {
            return Ok(install_path);
        }

        let tmp_dir = TempDir::new_in(cache.root())?;
        let archive_path = tmp_dir.path().join("archive");
        let size = download(&package.url, &archive_path, &package.sha256)?;
        if let Some(expected) = package.size {
            if expected != size {
                return Err(AppError::Cache {
                    message: format!("download size mismatch: expected {expected}, got {size}"),
                });
            }
        }

        fs::create_dir_all(&bin_dir)?;

        match package.format {
            PackageFormat::File => {
                if package.bin_paths.is_empty() {
                    fs::copy(&archive_path, &install_path)?;
                    make_executable(&install_path)?;
                } else if package.bin_paths.len() == 1 {
                    let target = bin_dir.join(bin_file_name(&package.bin_paths[0])?);
                    fs::copy(&archive_path, &target)?;
                    make_executable(&target)?;
                } else {
                    return Err(AppError::Cache {
                        message: "file package cannot define multiple bin_paths".to_string(),
                    });
                }
            }
            PackageFormat::TarGz => {
                let unpack_dir = tmp_dir.path().join("unpacked");
                fs::create_dir_all(&unpack_dir)?;
                unpack_tar_gz(&archive_path, &unpack_dir)?;
                install_from_unpack(&unpack_dir, &bin_dir, &package.bin_paths, "tar.gz")?;
            }
            PackageFormat::TarXz => {
                let unpack_dir = tmp_dir.path().join("unpacked");
                fs::create_dir_all(&unpack_dir)?;
                unpack_tar_xz(&archive_path, &unpack_dir)?;
                install_from_unpack(&unpack_dir, &bin_dir, &package.bin_paths, "tar.xz")?;
            }
            PackageFormat::Zip => {
                unpack_zip(&archive_path, &bin_dir)?;
                if package.bin_paths.is_empty() {
                    if !install_path.exists() {
                        return Err(AppError::Cache {
                            message: format!(
                                "bin_path not found in archive: {}",
                                install_path.display()
                            ),
                        });
                    }
                    make_executable(&install_path)?;
                } else {
                    for bin_path in package.bin_paths.iter().cloned() {
                        let expected = bin_dir.join(&bin_path);
                        if !expected.exists() {
                            return Err(AppError::Cache {
                                message: format!(
                                    "bin_path not found in archive: {}",
                                    expected.display()
                                ),
                            });
                        }
                        make_executable(&expected)?;
                    }
                }
            }
        }
        Ok(install_path)
    })
}

fn install_from_unpack(
    unpack_dir: &Path,
    bin_dir: &Path,
    bin_paths: &[String],
    format: &str,
) -> Result<(), AppError> {
    if bin_paths.is_empty() {
        return Err(AppError::Cache {
            message: format!("{format} package missing bin_paths"),
        });
    }

    for bin_path in bin_paths.iter().cloned() {
        let source = unpack_dir.join(&bin_path);
        if !source.exists() {
            return Err(AppError::Cache {
                message: format!("bin_path not found in archive: {}", source.display()),
            });
        }
        let target = bin_dir.join(bin_file_name(&bin_path)?);
        fs::copy(&source, &target)?;
        make_executable(&target)?;
    }

    Ok(())
}

fn bin_file_name(path: &str) -> Result<String, AppError> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .ok_or_else(|| AppError::Cache {
            message: format!("invalid bin_path: {path}"),
        })
}

fn primary_bin_path(
    cache: &Cache,
    tool: &str,
    version: &str,
    bin_paths: &[String],
) -> PathBuf {
    if let Some(bin_path) = find_bin_path(tool, bin_paths) {
        if let Ok(file_name) = bin_file_name(bin_path) {
            return cache.tool_bin_dir(tool, version).join(file_name);
        }
    }
    if let Some(first) = bin_paths.first() {
        if let Ok(file_name) = bin_file_name(first) {
            return cache.tool_bin_dir(tool, version).join(file_name);
        }
    }
    cache.tool_bin_path(tool, version)
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
    let response = ureq::get(url)
        .call()
        .map_err(|err| AppError::Cache {
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
