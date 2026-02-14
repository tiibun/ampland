# AGENTS

This project is implemented in Rust.

## Goals
- Build a fast, reliable tool manager with native shims.
- Keep configuration in a single user file.
- Provide deterministic version resolution based on cwd scope.

## Scope
- CLI, shim resolution, installer, and cache management.
- Cross-platform support (macOS, Linux, Windows).

## Non-Goals
- Project-local config files as the primary source of truth.
- Language-specific version managers.

## Implementation Notes
- Prefer a single static binary for the CLI.
- Shims are native executables on all platforms.
- Use file locks for cache writes.
- Avoid shell-specific behavior.

## Code Style
- Keep modules small and focused.
- Use explicit error types and context.
- Prefer readable, stable APIs over cleverness.

## Build
- `cargo build`
- `cargo build --release`

## Run
- `cargo run -- <args>`

## Formatting
- `cargo fmt`

## Lint
- `cargo clippy --all-targets --all-features -- -D warnings`

## Test
- `cargo test`

## Coverage
- `cargo llvm-cov --lcov --output-path lcov.info`
