# Integration Workspace

This directory is reserved for scenarios that span both `appfs` and `appfs-agent`.

## Intended Contents

- `fixtures/`: reusable mounted-tree examples and golden data
- `scripts/`: local bring-up helpers for mounts, bridges, and agent sessions
- `tests/`: end-to-end validation that exercises both layers together

## First Recommended Scenarios

1. Mount AppFS and verify `appfs-agent` can read `.res.jsonl` resources.
2. Trigger `*.act` writes from the agent runtime and verify resulting event streams.
3. Validate path conventions, append semantics, and long-running observation flows.

The goal is to keep shared integration assets here, not to duplicate each subproject's own unit or contract tests.
