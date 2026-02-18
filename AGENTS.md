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

## Working with AI Agents

### Planning Files
When an agent creates implementation plans or task lists, use these files:
- `PLAN.md` - High-level implementation plan
- `TASKS.md` - Detailed task checklist
- `.agent/` - Agent scratchpad directory

These files are `.gitignore`d to keep the codebase clean.

### Creating PRs with Plans
To preserve planning context in PRs while keeping it out of the codebase:

```bash
# Agent creates PLAN.md during implementation
# Use it as PR body when creating the PR
gh pr create --title "Feature: ..." --body-file PLAN.md

# Or combine with description
{
  echo "## Summary"
  echo "Brief description of changes"
  echo ""
  echo "## Implementation Plan"
  cat PLAN.md
} | gh pr create --title "Feature: ..." --body-file -
```

This workflow keeps planning context visible in PRs for reviewers while avoiding clutter in the repository.

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
