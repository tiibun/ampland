# Self-Update Command Design

**Date:** 2026-03-28
**Status:** Approved

## Overview

Add an `ampland update` command that allows ampland to update itself by downloading a new binary from GitHub Releases and replacing the current executable.

## Command Interface

```
ampland update              # Update to latest version (with confirmation)
ampland update 0.2.7        # Update to specific version (may downgrade, with confirmation)
ampland update --yes        # Skip confirmation prompt (for scripting)
```

Any leading `v` in the user-supplied version argument is stripped before use (e.g., `v0.2.7` → `0.2.7`).

## Execution Flow

1. Call GitHub Releases API to fetch release info (with `User-Agent: ampland/{version}` header)
2. Compare with current version (`env!("CARGO_PKG_VERSION")`)
3. If `version` is `None` and already at latest: print `already up to date (x.y.z)` and exit 0 (suppressed if `--quiet`)
4. If `version` is `Some(v)` and `v == current_version`: print `already at version x.y.z` and exit 0 (suppressed if `--quiet`)
5. Prompt (unless `--yes`; output suppressed if `--quiet`):
   - Upgrade or downgrade direction determined using `semver::Version::parse` comparison (not string comparison)
   - Upgrade: `update x.y.z -> a.b.c? [y/N]`
   - Downgrade: `downgrade x.y.z -> a.b.c? [y/N]`
   - Any input other than `y`/`Y`, including EOF (Ctrl+D), is treated as "no" and exits 0
6. Get current binary path via `std::env::current_exe()?.canonicalize()` to resolve symlinks
7. Determine asset name from platform/arch (e.g. `ampland-macos-arm64`)
8. Download `.sha256` sidecar asset and parse expected hex digest (bare hex, no filename)
9. Download binary, computing SHA-256 in a single streaming pass during download
10. Compare computed digest with expected; abort and delete temp file on mismatch
11. On Unix: set executable permissions (`0o755`)
12. `std::fs::rename` to atomically replace the current binary

## Implementation

### New file: `src/updater.rs`

```rust
struct Release { tag_name: String, assets: Vec<Asset> }
struct Asset { name: String, browser_download_url: String }

pub fn self_update(version: Option<&str>, yes: bool) -> Result<(), AppError>
fn fetch_release(version: Option<&str>) -> Result<Release, AppError>
fn asset_name_for_current_target() -> Result<String, AppError>
fn download_with_hash(url: &str, dest: &Path) -> Result<String, AppError>
// Returns computed hex digest; writes binary to dest in a single streaming pass
fn replace_binary(temp_path: &Path, target: &Path) -> Result<(), AppError>
```

### `asset_name_for_current_target()`

Reuses `Target::current()` from `src/manifest.rs` to map to release asset names via exact case-sensitive string equality against `asset.name`:

| platform | arch  | asset name              |
|----------|-------|-------------------------|
| macos    | arm64 | ampland-macos-arm64     |
| macos    | x64   | ampland-macos-x64       |
| linux    | x64   | ampland-linux-x64       |
| windows  | x64   | ampland-windows-x64.exe |

### `.sha256` sidecar format

Each release asset has a companion `{asset_name}.sha256` file containing **only** the lowercase hex digest (no filename, no newline other than a trailing one):

```
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
```

Generated in CI with:
```sh
shasum -a 256 ampland-macos-arm64 | awk '{print $1}' > ampland-macos-arm64.sha256
```

`fetch_release()` locates the sidecar URL from the same `Release` object by looking for an asset named `{asset_name}.sha256`.

### `download_with_hash()`

Downloads the binary to `dest` while streaming through `sha2::Sha256`, returning the final hex digest. This is a single pass — consistent with `installer.rs`. The caller compares the returned digest against the value from the sidecar file.

### Temp file handling

- Derive the temp file path as `{canonical_exe_dir}/{random_name}.tmp`
- Using `canonical_exe_dir` (from `current_exe()?.canonicalize()?.parent()`) ensures the temp file and the target are on the same filesystem, enabling atomic `rename`
- If temp file creation fails with a permission error, report: `cannot write to {dir}: permission denied (try sudo)`
- If `rename` fails after download, delete the temp file and surface the error

### `replace_binary()`

- **Unix and Windows**: `std::fs::rename(temp, target)` — on Windows, Rust's `std::fs::rename` calls `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` internally, which handles replacing an existing file. If the call fails because the binary is locked (Windows returns `ERROR_ACCESS_DENIED`), surface an error: `cannot replace running binary on Windows; download the new version manually: https://github.com/tiibun/ampland/releases/tag/v{version}`

### CLI changes

- `src/cli.rs`: Add `Command::Update { version: Option<String>, yes: bool }`
- `src/main.rs`: Handle `Command::Update` and call `updater::self_update()`
- Output in `Command::Update` is gated on `!cli.quiet` consistent with all other commands

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Network error | `AppError::Other` with descriptive message |
| GitHub API rate limit (403/429) | `AppError::Other`: `GitHub API rate limit exceeded; try again later` |
| Write permission denied (temp creation) | `AppError::Other` with message suggesting `sudo` |
| `rename` fails | Delete temp file, `AppError::Other` with OS error detail |
| Version not found | `AppError::Other`: `release v0.x.x not found on GitHub` |
| Sidecar asset missing | `AppError::Other`: `no checksum file found for {asset_name} in release v{version}` |
| SHA-256 mismatch | Delete temp file, `AppError::Other`: `checksum mismatch: download may be corrupted` |
| Already up to date | Print message (unless `--quiet`), exit 0 |
| Windows binary locked | `AppError::Other` with release URL for manual download |
| User declines prompt | Print nothing, exit 0 |

## Dependencies

No new dependencies required. Uses existing:
- `ureq` — HTTP requests
- `semver` — version comparison
- `sha2` — SHA-256 streaming hash
- `serde_json` — parse GitHub API response

## GitHub API

- Latest: `GET https://api.github.com/repos/tiibun/ampland/releases/latest`
- Specific: `GET https://api.github.com/repos/tiibun/ampland/releases/tags/v{version}`
- All requests must include header: `User-Agent: ampland/{current_version}`
- No authentication required for public repo (60 unauthenticated requests/hour/IP)

## Release Workflow Changes

Each release must publish sidecar `.sha256` files alongside each binary asset:

```
ampland-macos-arm64
ampland-macos-arm64.sha256
ampland-macos-x64
ampland-macos-x64.sha256
ampland-linux-x64
ampland-linux-x64.sha256
ampland-windows-x64.exe
ampland-windows-x64.exe.sha256
```

The `.sha256` file is generated immediately after building the binary in CI and uploaded in the same step, before any cross-job release creation to avoid race conditions.
