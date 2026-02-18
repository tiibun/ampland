## ampland

ampland is a tool manager that keeps configuration in a single user file and
installs tool versions into a user cache, with shims that dispatch to the
selected binaries.

## Installation

### Download from releases

#### macOS

For Apple Silicon:

```sh
curl -L https://github.com/tiibun/ampland/releases/latest/download/ampland-macos-arm64 -o ampland
chmod +x ampland
sudo mv ampland /usr/local/bin/
```

For Intel Mac:

```sh
curl -L https://github.com/tiibun/ampland/releases/latest/download/ampland-macos-x64 -o ampland
chmod +x ampland
sudo mv ampland /usr/local/bin/
```

Or install to a user directory:

```sh
mkdir -p ~/.local/bin
mv ampland ~/.local/bin/
# Ensure ~/.local/bin is in your PATH
```

#### Linux

```sh
# For x86_64
curl -L https://github.com/tiibun/ampland/releases/latest/download/ampland-linux-x64 -o ampland

# Make it executable
chmod +x ampland

# Move to a directory in your PATH
sudo mv ampland /usr/local/bin/
```

Or install to a user directory:

```sh
mkdir -p ~/.local/bin
mv ampland ~/.local/bin/
# Ensure ~/.local/bin is in your PATH
```

#### Windows

Download the latest release from:
```
https://github.com/tiibun/ampland/releases/latest
```

Download `ampland-windows-x64.exe`, rename it to `ampland.exe`, and place it in a directory in your PATH.

Or use PowerShell:

```powershell
# Download the latest release
Invoke-WebRequest -Uri "https://github.com/tiibun/ampland/releases/latest/download/ampland-windows-x64.exe" -OutFile "$env:LOCALAPPDATA\ampland\ampland.exe"

# Add to PATH (optional, if not already in PATH)
[Environment]::SetEnvironmentVariable(
	"PATH",
	"$env:LOCALAPPDATA\ampland;" + [Environment]::GetEnvironmentVariable("PATH", "User"),
	"User"
)
```

### Build from source

```
cargo build --release
```

The binary will be at `target/release/ampland`.

### Run without installing

```
cargo run -- <args>
```

## Quick start

Initialize PATH via `ampland activate` before running commands.

To make it permanent, add it to your shell rc file or the Windows user PATH.

macOS and Linux (bash/zsh):

```
eval "$(ampland activate)"
```

Add to `~/.bashrc` or `~/.zshrc`:

```sh
echo 'eval "$(ampland activate)"' >> ~/.bashrc
```

macOS and Linux (fish):

```
eval (ampland activate)
```

Add to `~/.config/fish/config.fish`:

```fish
echo 'eval (ampland activate)' >> ~/.config/fish/config.fish
```

Windows (PowerShell):

```
$env:Path = "$env:LOCALAPPDATA\ampland\shims;$env:Path"
```

Add to your PowerShell profile:

```powershell
Add-Content $PROFILE '$env:Path = "$env:LOCALAPPDATA\ampland\shims;$env:Path"'
```

Or add it to the Windows user PATH (persistent):

```powershell
[Environment]::SetEnvironmentVariable(
	"PATH",
	"$env:LOCALAPPDATA\ampland\shims;" + [Environment]::GetEnvironmentVariable("PATH", "User"),
	"User"
)
```

Example workflow to set a tool for the current directory and verify the resolved executable:

```
ampland use node 22.1.0
node --version
```

Common commands:

- `ampland use <tool> <version>`
- `ampland install <tool> <version>`
- `ampland list`
- `ampland search [query]`
- `ampland doctor`

If you omit the version in `ampland install <tool>`, ampland installs the
latest version from the manifest for the current platform and architecture.

You can also pass a combined spec like `ampland install node@24`.

`ampland doctor` also reports whether the shims directory is early in `PATH`.

Concepts:
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

If `manifest.url` is not set, ampland uses the default release URL:
`https://github.com/tiibun/ampland/releases/latest/download/installers.toml`.

## Manifest signing

Generate a keypair and public key hex:

```
openssl genpkey -algorithm ed25519 -out manifest_ed25519.pem
openssl pkey -in manifest_ed25519.pem -pubout -outform DER | tail -c 32 | xxd -p -c 64 > manifest_public_key.hex
```

Use the public key hex in one of these places:
- `manifest.public_key` in `config.toml`.
- `DEFAULT_PUBLIC_KEY_HEX` in the source.

For GitHub Actions, store the PEM contents in `MANIFEST_SIGNING_KEY`.

Local publish script:

```
MANIFEST_SIGNING_KEY="$(cat manifest_ed25519.pem)" ./scripts/publish-manifest.sh
```

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
- `ampland doctor`
- `ampland which <tool>`
- `ampland explain <tool>`
- `ampland shim rebuild`

Common flags:
- `--path <dir>`: resolve versions as if the current directory were `<dir>`.
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
- Use file locks during install to avoid concurrent writes to the cache.
