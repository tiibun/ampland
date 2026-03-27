# Self-Update Command Design

**Date:** 2026-03-28
**Status:** Approved

## Overview

Add an `ampland update` command that allows ampland to update itself by downloading a new binary from GitHub Releases and replacing the current executable.

## Command Interface

```
ampland update              # Update to latest version (with confirmation)
ampland update 0.2.7        # Update to specific version (with confirmation)
ampland update --yes        # Skip confirmation prompt (for scripting)
```

## Execution Flow

1. Call GitHub Releases API to fetch release info
2. Compare with current version (`env!("CARGO_PKG_VERSION")`)
3. If already up to date: print `already up to date (x.y.z)` and exit
4. Prompt: `update x.y.z -> a.b.c? [y/N]` (skip if `--yes`)
5. Get current binary path via `std::env::current_exe()`
6. Determine asset name from platform/arch (e.g. `ampland-macos-arm64`)
7. Download to temp file in same directory → atomic `rename` replace

## Implementation

### New file: `src/updater.rs`

```rust
struct Release { tag_name: String, assets: Vec<Asset> }
struct Asset { name: String, browser_download_url: String }

pub fn self_update(version: Option<&str>, yes: bool) -> Result<(), AppError>
fn fetch_release(version: Option<&str>) -> Result<Release, AppError>
fn asset_name_for_current_target() -> Result<String, AppError>
fn download_and_replace(url: &str, current_exe: &Path) -> Result<(), AppError>
```

### `asset_name_for_current_target()`

Reuses `Target::current()` from `src/manifest.rs` to map to release asset names:

| platform | arch  | asset name              |
|----------|-------|-------------------------|
| macos    | arm64 | ampland-macos-arm64     |
| macos    | x64   | ampland-macos-x64       |
| linux    | x64   | ampland-linux-x64       |
| windows  | x64   | ampland-windows-x64.exe |

### `download_and_replace()`

- Write binary to a temp file in the same directory as the current exe
- On Unix: set executable permissions (`0o755`)
- `std::fs::rename` to atomically replace the current binary

### CLI changes

- `src/cli.rs`: Add `Command::Update { version: Option<String>, yes: bool }`
- `src/main.rs`: Handle `Command::Update` and call `updater::self_update()`

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Network error | `AppError::Cache` with descriptive message |
| Write permission denied | Error message suggesting `sudo` |
| Version not found | `release v0.x.x not found on GitHub` |
| Already up to date | Print message, exit 0 |

## Dependencies

No new dependencies. Uses existing:
- `ureq` — HTTP requests
- `semver` — version comparison
- `serde_json` — parse GitHub API response

## GitHub API

- Latest: `GET https://api.github.com/repos/tiibun/ampland/releases/latest`
- Specific: `GET https://api.github.com/repos/tiibun/ampland/releases/tags/v{version}`
- No authentication required for public repo
