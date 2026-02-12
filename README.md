## ampland

ampland is a tool manager that keeps configuration in a single user file and
installs tool versions into a user cache, with shims that dispatch to the
selected binaries.

Key ideas:
- Configuration lives in:
	- macOS: `~/Library/Application Support/ampland/config.toml`
	- Linux: `~/.config/ampland/config.toml`
- Installed tool versions live under `~/.local/ampland/cache/`.
- Shims are created so `tool` resolves to the right version based on the
	current working directory.

Windows paths:
- Config: `%APPDATA%\ampland\config.toml`
- Cache: `%LOCALAPPDATA%\ampland\cache`
- Shims: `%LOCALAPPDATA%\ampland\shims`

On Windows, shims should be native binaries (not `.cmd`) to avoid issues with
`child_process.spawn` in VS Code extensions and other tooling.

## Configuration

ampland uses a global section and optional scoped overrides based on the
current working directory.

Example `~/Library/Application Support/ampland/config.toml`:

```toml
[global.tools]
node = "20.12.0"
python = "3.12.1"

[manifest]
url = "https://github.com/ORG/REPO/releases/latest/download/installers.toml"
public_key = "HEX_ENCODED_ED25519_PUBLIC_KEY"
ttl_hours = 24

[[scope]]
path = "/Users/toshi/work/ampland/**"
[scope.tools]
node = "22.1.0"

[[scope]]
path = "/Users/toshi/work/legacy/**"
[scope.tools]
node = "16.20.2"
```

Resolution order:
1. Find all `scope` entries whose `path` glob matches the current directory.
2. Pick the most specific match (longest path pattern).
3. Use tools from that scope; fall back to `[global.tools]` for missing tools.

## Installation layout

```
~/.local/ampland/cache/
	node/22.1.0/bin/node
	python/3.12.1/bin/python
~/.local/ampland/shims/
	node
	python
```

Windows layout:

```
%LOCALAPPDATA%\ampland\cache\
	node\22.1.0\bin\node.exe
	python\3.12.1\bin\python.exe
%LOCALAPPDATA%\ampland\shims\
	node.exe
	python.exe
```

Shims are tiny executables that resolve the correct version and exec the
selected binary.

## Installer manifest

ampland uses a signed TOML manifest that maps tools and versions to download
URLs. A minimal manifest is embedded in the binary, and ampland will
optionally fetch and cache a newer manifest if `manifest.url` and
`manifest.public_key` are set.

Manifest behavior:
- The cached manifest is refreshed when the TTL has expired.
- Signature verification uses Ed25519 with a public key provided in config.
- If remote fetch or verification fails, ampland falls back to the cached or
	embedded manifest.

Manifest fields:
- `manifest.url`: URL to `installers.toml`.
- `manifest.sig_url`: optional; defaults to `installers.toml.sig` at the same
	base URL.
- `manifest.public_key`: hex-encoded Ed25519 public key.
- `manifest.ttl_hours`: cache TTL in hours (default 24).

## Export and import

Export the resolved tool versions for the current directory:

```
ampland export --path .
```

Import a previously exported file and bind it to a path scope:

```
ampland import --path /Users/toshi/work/ampland ./ampland.lock.toml
```

Suggested usage:
- Commit an exported lock file for CI or team sharing (optional).
- Keep `config.toml` as the source of truth for local rules.

## CLI

```
ampland <command> [flags]
```

Core commands:
- `ampland use <tool> <version> [--path <dir>]`
- `ampland install <tool> <version>`
- `ampland uninstall <tool> <version>`
- `ampland search [query]`
- `ampland list`
- `ampland gc`
- `ampland update-manifest`
- `ampland export --path <dir> [--format toml|json]`
- `ampland import --path <dir> <file>`
- `ampland doctor`
- `ampland which <tool>`
- `ampland explain <tool>`
- `ampland shim rebuild`

Common flags:
- `--path <dir>`: resolve versions as if the current directory were `<dir>`.
- `--format <toml|json>`: output format for `export`.
- `--json`: print machine-readable output where available.
- `--quiet`: only print errors.
- `--verbose`: print resolution and install details.

Exit codes:
- `0`: success
- `1`: generic error
- `2`: invalid arguments
- `3`: tool not installed
- `4`: config parse or resolution error
- `5`: cache or filesystem error

## Diagnostics

```
ampland doctor
ampland which node
ampland explain node
```

`doctor` checks PATH conflicts, missing versions, and cache integrity. `which`
shows the resolved binary. `explain` prints the matching scope and fallback
chain used to decide a version.

## Notes and safety

- PATH order matters. Put `~/.local/ampland/shims` early in PATH and avoid
	conflicts with other managers (asdf, mise, pyenv, etc.).
- Use a lock file or export/import flow for reproducible builds in CI.
- Use file locks during install to avoid concurrent writes to the cache.
