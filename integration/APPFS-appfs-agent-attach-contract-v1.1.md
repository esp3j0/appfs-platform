# AppFS x appfs-agent Attach Contract v1.1

## Status

Phase 1 implemented in the integration workspace. Phase 2 launcher work is reserved.

## Purpose

This document defines the stable attach contract between `appfs` and `appfs-agent`.

The goal is to stop treating `/_appfs` directory discovery as the primary protocol. From v1.1 onward:

1. AppFS publishes a versioned runtime manifest at `/.well-known/appfs/runtime.json`;
2. `appfs-agent` prefers explicit attach inputs over directory heuristics;
3. multi-agent attach is a first-class model, with one shared runtime session and distinct agent attach identities.

## Source Of Truth

For this contract:

1. `appfs-platform/integration/` is the source of truth for the cross-project attach contract and checkpoints;
2. standalone `appfs` remains the source of truth for AppFS component code;
3. standalone `appfs-agent` remains the source of truth for agent component code.

## Identity Model

The default runtime model is:

1. one AppFS mount/runtime publishes one shared runtime manifest;
2. `runtime_session_id` identifies that shared AppFS runtime;
3. each `appfs-agent` process gets its own `attach_id`;
4. `APPFS_AGENT_ROLE` is descriptive only and does not need to be unique.

This means multiple agents may safely attach to the same mount at the same time, as long as they keep distinct `attach_id` values.

## Stable Surface v1.1

### C0. Mount Bring-Up

The baseline bring-up path is:

1. initialize an AppFS database;
2. start AppFS with `agentfs appfs up`;
3. wait for the mounted control plane and runtime manifest to appear.

Required visible paths:

1. `/_appfs/register_app.act`
2. `/_appfs/list_apps.act`
3. `/.well-known/appfs/runtime.json`

### C1. Runtime Manifest

AppFS must publish a runtime manifest at:

1. `/.well-known/appfs/runtime.json`

Required manifest fields:

1. `schema_version: 1`
2. `runtime_kind: "appfs"`
3. `mount_root`
4. `runtime_session_id`
5. `managed`
6. `multi_agent_mode: "shared_mount_distinct_attach"`
7. `control_plane.register_action`
8. `control_plane.unregister_action`
9. `control_plane.list_action`
10. `control_plane.registry`
11. `control_plane.events`
12. `capabilities.app_registration`
13. `capabilities.event_stream`
14. `capabilities.multi_app`
15. `capabilities.scope_switch`
16. `capabilities.multi_agent_attach`
17. `generated_at`

The manifest describes the shared runtime only. It must not contain per-agent state.

### C2. Attach Input Precedence

`appfs-agent` attach resolution order is:

1. environment variables
2. runtime manifest
3. directory heuristic fallback

Phase 1 attach environment variables are:

1. `APPFS_ATTACH_SCHEMA=1`
2. `APPFS_RUNTIME_MANIFEST`
3. `APPFS_MOUNT_ROOT`
4. `APPFS_RUNTIME_SESSION_ID`
5. `APPFS_ATTACH_ID`
6. `APPFS_AGENT_ROLE`

If `APPFS_ATTACH_ID` is omitted, `appfs-agent` may generate a local ephemeral attach id for status display. This does not create shared runtime state.

### C3. Status Surface

When `appfs-agent` detects an AppFS attach, `/status` text and JSON output must surface:

1. `appfs.detected`
2. `appfs.attach_source`
3. `appfs.mount_root`
4. `appfs.runtime_session_id`
5. `appfs.attach_id`
6. `appfs.attach_role`
7. `appfs.multi_agent_mode`

`appfs-agent` must remain usable outside AppFS. In non-AppFS workspaces, `/status` must still report `appfs.detected = false`.

### C4. Registered App Loop

The mounted app tree must continue to support the existing runtime loop:

1. register one app through `/_appfs/register_app.act`
2. read `*.res.jsonl` snapshot resources
3. append `*.act` action requests
4. observe results through `_stream/events.evt.jsonl`

### C5. Multi-Agent Attach Semantics

Phase 1 multi-agent rules are:

1. many agents may share the same `mount_root`
2. many agents may share the same `runtime_session_id`
3. each attached agent should use a distinct `attach_id`
4. no shared attached-agent registry is written in Phase 1

This avoids cross-agent write contention while keeping runtime identity explicit.

## Acceptance Checkpoints

### IC-0. Mounted Workspace Attach Baseline

Goal:

1. prove AppFS can mount, publish the runtime manifest, and host a workspace that `appfs-agent` can inspect through `/status`.

Required clauses:

1. `C0. Mount Bring-Up`
2. `C1. Runtime Manifest`
3. `C2. Attach Input Precedence`
4. `C3. Status Surface`

Current gate:

1. `integration/scripts/test-windows-appfs-agent-smoke.ps1`

### IC-1. Registered App Loop Baseline

Goal:

1. prove the AppFS app loop still works end-to-end while the runtime manifest contract is present.

Required clauses:

1. `C0. Mount Bring-Up`
2. `C1. Runtime Manifest`
3. `C4. Registered App Loop`

Current gate:

1. `integration/scripts/test-windows-appfs-agent-http-demo.ps1`

### IC-2. Multi-Agent Attach Baseline

Goal:

1. run at least two `appfs-agent` processes against the same mount and confirm they share `runtime_session_id` while keeping distinct `attach_id` values.

Not yet automated in Phase 1.

## Non-Goals For v1.1

Phase 1 does not yet require:

1. named pipes, local sockets, or heartbeat RPC;
2. AppFS-side attached-agent registry files;
3. launcher-driven child agent orchestration;
4. replacing OS-level sandboxing with AppFS;
5. removing heuristic detection entirely.

`/_appfs` remains an internal control plane path, but it is no longer the primary attach protocol.

## Failure Buckets

When the integration run fails, classify it into one of these buckets first:

1. mount bring-up failure;
2. runtime manifest publication failure;
3. attach resolution failure inside `appfs-agent`;
4. status surface regression;
5. app registration or tree materialization failure;
6. snapshot read failure;
7. action append failure;
8. event observation failure.

## Next Work

The next planned step after this contract is:

1. keep `IC-0` and `IC-1` green as mandatory baselines;
2. add `IC-2` multi-agent automation;
3. implement Phase 2 launcher-driven startup that injects attach env explicitly;
4. keep directory heuristic detection only as a compatibility fallback.

Future design note:

1. principal-aware shared/private app visibility is intentionally deferred and tracked in `appfs/docs/plans/2026-04-07-appfs-principal-visibility-and-agent-identity.md`.
