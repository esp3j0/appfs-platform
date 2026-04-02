# appfs-platform

Monorepo for the AppFS stack:

- `appfs/`: the filesystem protocol, runtime, bridges, SDKs, and contract suites
- `appfs-agent/`: the agent runtime that operates on top of AppFS
- `integration/`: cross-project scripts, fixtures, and end-to-end test scaffolding

## Why This Repo Exists

`appfs` and `appfs-agent` are separate layers, but they now evolve together:

- AppFS defines the filesystem-native app contract
- `appfs-agent` consumes that contract as an agent runtime
- integration work frequently changes both layers at once

This monorepo keeps those layers separate in code layout while making joint development, CI, and end-to-end testing easier.

## Layout

```text
.
├── appfs/
├── appfs-agent/
├── docs/
│   └── adr/
└── integration/
    ├── fixtures/
    ├── scripts/
    └── tests/
```

## Recommended Workflow

Work in the subproject that owns the change:

- AppFS protocol, mount/runtime, bridges, adapters: `appfs/`
- agent runtime, tools, sessions, hooks, providers: `appfs-agent/`
- end-to-end mount + agent scenarios: `integration/`

Prefer adding integration assets only when a scenario truly spans both systems.

## Initial CI Scope

The root CI currently covers:

- `appfs-agent` Rust workspace on Ubuntu, Windows, and macOS
- `appfs-agent` repository-level Python tests
- `appfs` core Linux contract gate

This keeps the combined repo practical to work in while preserving the highest-signal checks for the two active layers.

## Current Import Points

This monorepo was initialized from the current working branches of the two source repositories:

- `appfs`: imported from the current `agentfs` HEAD
- `appfs-agent`: imported from the current `claw-code` HEAD

Future updates should happen in this repository.
