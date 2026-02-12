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

pub fn install(cache: &Cache, tool: &str, version: &str, package: &ResolvedPackage) -> Result<PathBuf, AppError> {
    cache.with_lock(|| {
        let install_path = cache.tool_bin_path(tool, version);
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

        if let Some(parent) = install_path.parent() {
            fs::create_dir_all(parent)?;
        }

        match package.format {
            PackageFormat::File => {
                fs::copy(&archive_path, &install_path)?;
            }
            PackageFormat::TarGz => {
                let unpack_dir = tmp_dir.path().join("unpacked");
                fs::create_dir_all(&unpack_dir)?;
                unpack_tar_gz(&archive_path, &unpack_dir)?;
                let source = match &package.bin_path {
                    Some(path) => unpack_dir.join(path),
                    None => {
                        return Err(AppError::Cache {
                            message: "tar.gz package missing bin_path".to_string(),
                        })
                    }
                };
                if !source.exists() {
                    return Err(AppError::Cache {
                        message: format!("bin_path not found in archive: {}", source.display()),
                    });
                }
                fs::copy(&source, &install_path)?;
            }
            PackageFormat::TarXz => {
                let unpack_dir = tmp_dir.path().join("unpacked");
                fs::create_dir_all(&unpack_dir)?;
                unpack_tar_xz(&archive_path, &unpack_dir)?;
                let source = match &package.bin_path {
                    Some(path) => unpack_dir.join(path),
                    None => {
                        return Err(AppError::Cache {
                            message: "tar.xz package missing bin_path".to_string(),
                        })
                    }
                };
                if !source.exists() {
                    return Err(AppError::Cache {
                        message: format!("bin_path not found in archive: {}", source.display()),
                    });
                }
                fs::copy(&source, &install_path)?;
            }
            PackageFormat::Zip => {
                let bin_dir = cache.tool_version_dir(tool, version).join("bin");
                fs::create_dir_all(&bin_dir)?;
                unpack_zip(&archive_path, &bin_dir)?;
                let expected = match &package.bin_path {
                    Some(path) => bin_dir.join(path),
                    None => install_path.clone(),
                };
                if !expected.exists() {
                    return Err(AppError::Cache {
                        message: format!("bin_path not found in archive: {}", expected.display()),
                    });
                }
            }
        }

        make_executable(&install_path)?;
        Ok(install_path)
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
