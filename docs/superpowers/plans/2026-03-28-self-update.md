# Self-Update Command Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `ampland update [version] [--yes]` command that downloads the latest (or specified) ampland binary from GitHub Releases, verifies its SHA-256 checksum, and atomically replaces the running executable.

**Architecture:** A new `src/updater.rs` module contains all update logic: fetching release info from the GitHub API, streaming download with SHA-256 verification, and atomic binary replacement. `src/cli.rs` gains a new `Update` variant; `src/main.rs` dispatches to it. The CI workflow gains a `.sha256` sidecar step alongside each binary upload.

**Tech Stack:** Rust, `ureq` (HTTP), `sha2` (streaming hash), `semver` (version comparison), `serde_json` (GitHub API JSON), `clap` (CLI). No new dependencies.

---

## File Map

| Action | Path | Responsibility |
|--------|------|----------------|
| Create | `src/updater.rs` | All self-update logic |
| Modify | `src/cli.rs` | Add `Command::Update` variant |
| Modify | `src/main.rs` | Add `mod updater`, handle `Command::Update` |
| Modify | `.github/workflows/build-binaries.yml` | Generate and upload `.sha256` sidecars |

---

### Task 1: Add `Command::Update` to CLI

**Files:**
- Modify: `src/cli.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/cli.rs` in a new `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn update_command_parses_no_args() {
        let cli = Cli::try_parse_from(["ampland", "update"]).expect("parse");
        assert!(matches!(cli.command, Command::Update { version: None, yes: false }));
    }

    #[test]
    fn update_command_parses_version() {
        let cli = Cli::try_parse_from(["ampland", "update", "0.2.7"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::Update { version: Some(ref v), yes: false } if v == "0.2.7"
        ));
    }

    #[test]
    fn update_command_parses_yes_flag() {
        let cli = Cli::try_parse_from(["ampland", "update", "--yes"]).expect("parse");
        assert!(matches!(cli.command, Command::Update { version: None, yes: true }));
    }

    #[test]
    fn update_command_parses_version_and_yes() {
        let cli = Cli::try_parse_from(["ampland", "update", "0.2.7", "--yes"]).expect("parse");
        assert!(matches!(
            cli.command,
            Command::Update { version: Some(ref v), yes: true } if v == "0.2.7"
        ));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -q 2>&1 | grep "update_command"
```
Expected: compile error — `Command::Update` does not exist yet.

- [ ] **Step 3: Add `Update` variant to `Command` enum in `src/cli.rs`**

After the `Config` variant (line 73), add:

```rust
    #[command(about = "Update ampland to a new version")]
    Update {
        /// Version to update to (default: latest)
        version: Option<String>,
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -q 2>&1 | grep "update_command"
```
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs
git commit -m "feat: add Update subcommand to CLI"
```

---

### Task 2: Create `src/updater.rs` — `asset_name_for_current_target()`

**Files:**
- Create: `src/updater.rs`

- [ ] **Step 1: Write the failing test**

Create `src/updater.rs` with only the stub and tests first:

```rust
use crate::error::AppError;

// ---- placeholder so tests compile ----
fn asset_name_for_target(platform: &str, arch: &str) -> Result<String, AppError> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
```

Also add `mod updater;` to `src/main.rs` (before other mod declarations).

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test updater::tests -q 2>&1
```
Expected: tests fail with `not yet implemented`.

- [ ] **Step 3: Implement `asset_name_for_target()`**

Replace the placeholder in `src/updater.rs`:

```rust
use crate::error::AppError;
use crate::manifest::Target;

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

pub fn asset_name_for_current_target() -> Result<String, AppError> {
    let t = Target::current()?;
    asset_name_for_target(&t.platform, &t.arch)
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test updater::tests -q 2>&1
```
Expected: 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/updater.rs src/main.rs
git commit -m "feat: add asset_name_for_current_target to updater"
```

---

### Task 3: `fetch_release()` — GitHub API

**Files:**
- Modify: `src/updater.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/updater.rs` inside the existing `tests` module. Note: `serve_once` takes a `String` so tests can construct dynamic bodies without leaking memory:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test updater::tests::fetch_release -q 2>&1
```
Expected: compile error — `fetch_release_from` not defined.

- [ ] **Step 3: Implement `Release`, `Asset`, and `fetch_release_from()`**

Add to `src/updater.rs`. Use the `ureq` 2.x pattern-match approach for status codes — with `ureq` 2.x, non-2xx responses arrive as `Err(ureq::Error::Status(code, response))`, so match on the variant directly:

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
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

    let release: Release = response
        .into_json()
        .map_err(|err| AppError::Other {
            message: format!("failed to parse release info: {err}"),
        })?;

    Ok(release)
}

pub fn fetch_release(version: Option<&str>) -> Result<Release, AppError> {
    fetch_release_from(version, "https://api.github.com")
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test updater::tests::fetch_release -q 2>&1
```
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/updater.rs
git commit -m "feat: add fetch_release to updater"
```

---

### Task 4: `download_with_hash()`

**Files:**
- Modify: `src/updater.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/updater.rs`:

```rust
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test updater::tests::download_with_hash -q 2>&1
```
Expected: compile error — `download_with_hash` not defined.

- [ ] **Step 3: Implement `download_with_hash()`**

Mirror the `download()` pattern in `installer.rs` (single streaming pass through `sha2::Sha256`):

```rust
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use sha2::{Digest, Sha256};

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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test updater::tests::download_with_hash -q 2>&1
```
Expected: 1 test passes.

- [ ] **Step 5: Commit**

```bash
git add src/updater.rs
git commit -m "feat: add download_with_hash to updater"
```

---

### Task 5: `replace_binary()`

**Files:**
- Modify: `src/updater.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module:

```rust
    #[test]
    fn replace_binary_swaps_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("ampland");
        std::fs::write(&target, b"old").expect("write old");

        let new_bin = temp.path().join("ampland.new.tmp");
        std::fs::write(&new_bin, b"new").expect("write new");

        replace_binary(&new_bin, &target).expect("replace");
        assert_eq!(std::fs::read(&target).expect("read"), b"new");
        assert!(!new_bin.exists());
    }

    #[test]
    fn replace_binary_errors_on_missing_temp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let target = temp.path().join("ampland");
        let missing = temp.path().join("missing.tmp");
        assert!(replace_binary(&missing, &target).is_err());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test updater::tests::replace_binary -q 2>&1
```
Expected: compile error — `replace_binary` not defined.

- [ ] **Step 3: Implement `replace_binary()`**

On Windows, `std::fs::rename` calls `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` internally. A locked binary produces `ERROR_SHARING_VIOLATION` = OS error **32** (not 5). Pass `new_ver` so the error message links to the correct release:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test updater::tests::replace_binary -q 2>&1
```
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/updater.rs
git commit -m "feat: add replace_binary to updater"
```

---

### Task 6: `self_update()` orchestration

**Files:**
- Modify: `src/updater.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module. `self_update_inner` accepts `out: &mut dyn IoWrite` for output and `input: &mut dyn BufRead` for the confirmation prompt so the full interactive path is testable without a real terminal:

```rust
    fn make_release_json(tag: &str, asset_url: &str, sha256_url: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","assets":[
                {{"name":"ampland-macos-arm64","browser_download_url":"{asset_url}"}},
                {{"name":"ampland-macos-arm64.sha256","browser_download_url":"{sha256_url}"}}
            ]}}"#
        )
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test updater::tests::self_update -q 2>&1
```
Expected: compile error — `self_update_inner` not defined.

- [ ] **Step 3: Implement `self_update_inner()` and `self_update()`**

```rust
use semver::Version;
use std::io::{BufRead, Write as IoWrite};

fn fetch_text(url: &str) -> Result<String, AppError> {
    ureq::get(url)
        .set("User-Agent", &format!("ampland/{CURRENT_VERSION}"))
        .call()
        .map_err(|err| AppError::Other {
            message: format!("failed to fetch {url}: {err}"),
        })?
        .into_string()
        .map_err(|err| AppError::Other {
            message: format!("failed to read response from {url}: {err}"),
        })
}

fn self_update_inner(
    version: Option<&str>,
    yes: bool,
    api_base: &str,
    out: &mut dyn IoWrite,
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
```

- [ ] **Step 4: Run all updater tests**

```bash
cargo test updater:: -q 2>&1
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/updater.rs
git commit -m "feat: add self_update orchestration to updater"
```

---

### Task 7: Wire `Command::Update` into `src/main.rs`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Add `Command::Update` handler to `run()` in `src/main.rs`**

Inside the `match cli.command { ... }` block, add after the `Config` arm:

```rust
        Command::Update { version, yes } => {
            updater::self_update(version.as_deref(), yes, cli.quiet)?;
        }
```

- [ ] **Step 2: Build to verify it compiles**

```bash
cargo build -q 2>&1
```
Expected: builds without errors.

- [ ] **Step 3: Run all tests**

```bash
cargo test -q 2>&1
```
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire update command into main"
```

---

### Task 8: Update CI workflow to publish `.sha256` sidecars

**Files:**
- Modify: `.github/workflows/build-binaries.yml`

- [ ] **Step 1: Add `.sha256` generation steps**

The hash command differs by OS: use `sha256sum` on Linux (`ubuntu-latest`) and `shasum -a 256` on macOS (`macos-latest`). Both commands produce the same output format, and the `awk '{print $1}'` strips the filename field.

After the "Prepare release asset (non-Windows)" step, add two steps:

```yaml
      - name: Generate SHA-256 sidecar (Linux)
        if: runner.os == 'Linux'
        run: |
          sha256sum "dist/${{ matrix.artifact_name }}" | awk '{print $1}' > "dist/${{ matrix.artifact_name }}.sha256"

      - name: Generate SHA-256 sidecar (macOS)
        if: runner.os == 'macOS'
        run: |
          shasum -a 256 "dist/${{ matrix.artifact_name }}" | awk '{print $1}' > "dist/${{ matrix.artifact_name }}.sha256"
```

After the "Prepare release asset (Windows)" step, add:

```yaml
      - name: Generate SHA-256 sidecar (Windows)
        if: runner.os == 'Windows'
        shell: pwsh
        run: |
          $hash = (Get-FileHash "dist/${{ matrix.artifact_name }}" -Algorithm SHA256).Hash.ToLower()
          Set-Content -Path "dist/${{ matrix.artifact_name }}.sha256" -Value $hash -NoNewline
```

Update the "Upload artifact" step to include the sidecar:

```yaml
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact_name }}
          path: |
            dist/${{ matrix.artifact_name }}
            dist/${{ matrix.artifact_name }}.sha256
          if-no-files-found: error
```

Update the "Create GitHub release" step to include the sidecar:

```yaml
      - name: Create GitHub release
        if: startsWith(github.ref, 'refs/tags/v')
        uses: softprops/action-gh-release@v2
        with:
          files: |
            dist/${{ matrix.artifact_name }}
            dist/${{ matrix.artifact_name }}.sha256
```

- [ ] **Step 2: Verify the workflow YAML is valid**

```bash
cat .github/workflows/build-binaries.yml
```
Expected: well-formed YAML with separate Linux/macOS/Windows sidecar steps and both binary and sidecar in the `files` list.

- [ ] **Step 3: Run all tests one final time**

```bash
cargo test -q 2>&1
```
Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/build-binaries.yml
git commit -m "ci: publish sha256 sidecar files alongside release binaries"
```

---

## Verification

After all tasks are complete:

```bash
# Build locally
cargo build -q

# Check command appears in help
./target/debug/ampland update --help
# Expected: shows "Update ampland to a new version" with [VERSION] and --yes flag

# Run full test suite
cargo test 2>&1
# Expected: all tests pass
```
