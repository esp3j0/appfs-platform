# CLAW.md

This file provides working guidance for the `appfs-agent` repository.

## Repository intent

- This repository is now positioned as the agent runtime companion for AppFS.
- The active implementation work happens in `rust/`.
- Some internal names still use `claw`; treat those as transitional implementation names, not the final product direction.

## Detected stack

- Languages: Rust, Python
- Primary runtime surface: Rust
- Supporting analysis/parity surface: Python

## Verification

- Run Rust verification from `rust/`
- Preferred checks:
  - `cargo build --workspace`
  - `cargo test --workspace`
- On Windows, interpret test failures carefully: some MCP stdio tests are still Unix-oriented

## Repository shape

- `rust/` contains the active runtime workspace that will evolve into AppFS's agent runtime
- `src/` contains parity-analysis and migration-support code
- `tests/` contains validation for the non-Rust workspace surfaces
- `PARITY.md` tracks migration status against the archived TS snapshot

## Working agreement

- Prefer changes that move the runtime toward an AppFS-native agent lifecycle
- Keep documentation honest when naming and implementation are temporarily out of sync
- Avoid broad rename churn unless it directly helps the next runtime milestone
- Update `README.md`, `rust/README.md`, and `PARITY.md` together when the product direction materially changes
