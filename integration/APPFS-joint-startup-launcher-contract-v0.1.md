# AppFS Joint Startup / Launcher Contract v0.1

**Status:** Draft for next implementation phase
**Scope:** Explicit startup path for `appfs` + `appfs-agent` after attach contract v1.1
**Updated:** 2026-04-07

## 1. Purpose

The attach contract is now stable enough that `appfs-agent` can identify an AppFS runtime explicitly through:

1. runtime manifest;
2. attach environment variables;
3. heuristic fallback only as compatibility behavior.

What is still missing is a supported startup path that makes this explicit at process launch time.

Today the common flow is still:

1. start AppFS separately;
2. wait for the mount and manifest;
3. manually start `appfs-agent`;
4. optionally inject `APPFS_ATTACH_*` by hand.

This document defines the next supported shape:

1. a launcher owns startup sequencing;
2. the launcher waits for AppFS runtime readiness;
3. the launcher starts `appfs-agent` against an AppFS-backed workspace;
4. the launcher injects attach inputs explicitly instead of relying on directory guessing.

## 2. Design Goals

The first launcher contract should:

1. make AppFS attach explicit at process start;
2. keep `appfs-agent` generic outside AppFS;
3. support one shared runtime with one or many attached agents;
4. remain compatible with future overlay-backed "start from existing directory" flows;
5. avoid requiring platform-specific same-path mount behavior.

## 3. Non-Goals

This phase does not yet attempt to:

1. define principal-aware or private/shared app visibility;
2. replace OS-level sandboxing with AppFS security semantics;
3. require in-place mounting over an already existing directory path;
4. introduce an AppFS-side attached-agent registry;
5. define connector account isolation rules.

## 4. Core Terms

For this document:

1. **launcher** means the process that coordinates AppFS runtime bring-up and child agent startup;
2. **runtime view** means the mounted AppFS filesystem path exposed to the agent process;
3. **host entry directory** means an optional existing host directory that may later become an overlay base;
4. **workspace cwd** means the child process working directory inside the runtime view;
5. **agent spec** means one child agent launch request, including its `attach_id`, optional role, and executable command.

## 5. Supported Runtime Model

The supported model for the first launcher phase is:

1. one launcher prepares one AppFS runtime;
2. one AppFS runtime publishes one shared `runtime_session_id`;
3. one or many `appfs-agent` child processes may attach to that runtime;
4. every child gets its own distinct `attach_id`;
5. every child runs inside the AppFS runtime view, not against a raw host path.

This means the launcher is not just "starting another process". It is creating a clear relationship:

1. shared runtime identity comes from AppFS;
2. child agent identity comes from the launcher;
3. workspace location comes from the mounted runtime view.

## 6. Startup Responsibilities

### 6.1 Launcher Responsibilities

The launcher must:

1. resolve the AppFS database or runtime identity it is responsible for;
2. start or attach to the AppFS runtime bring-up path;
3. wait until the AppFS control plane and runtime manifest are ready;
4. read `/.well-known/appfs/runtime.json`;
5. choose a child workspace cwd inside the mount root;
6. start each child with explicit attach environment variables;
7. shut children down cleanly when the launcher-owned runtime is torn down.

### 6.2 AppFS Responsibilities

AppFS must:

1. publish a correct runtime manifest before child agents are launched;
2. expose a stable mount root and control plane;
3. keep `runtime_session_id` stable for the lifetime of that runtime instance.

### 6.3 appfs-agent Responsibilities

`appfs-agent` must:

1. prefer explicit attach environment variables over manifest or heuristics;
2. surface attach metadata through `/status`;
3. remain usable outside AppFS when no launcher or attach inputs are present.

## 7. Child Launch Contract

For each child agent, the launcher must provide:

1. `APPFS_ATTACH_SCHEMA=1`
2. `APPFS_RUNTIME_MANIFEST=<absolute path>`
3. `APPFS_MOUNT_ROOT=<absolute path>`
4. `APPFS_RUNTIME_SESSION_ID=<manifest value>`
5. `APPFS_ATTACH_ID=<launcher-assigned instance id>`
6. optional `APPFS_AGENT_ROLE=<descriptive role>`

Rules:

1. the launcher should derive `APPFS_RUNTIME_SESSION_ID` from the runtime manifest, not from user guesswork;
2. if multiple children are launched, they must share the same manifest path, mount root, and runtime session id;
3. if multiple children are launched, each child must receive a distinct `APPFS_ATTACH_ID`;
4. the launcher should treat an env/manifest mismatch as a launcher bug, not as normal behavior.

## 8. Workspace Contract

The launcher should treat "AppFS-backed workspace" as a first-class concept.

For the first supported phase:

1. each child process `cwd` must be inside the AppFS mount root;
2. the default child cwd should be `<mount_root>/workspace`;
3. if the launcher allows overriding the workspace path, that override must still resolve inside the mount root;
4. the launcher may create the default workspace directory if it does not exist yet.

This keeps the product model clear:

1. AppFS provides the runtime view;
2. `appfs-agent` runs inside that view;
3. future overlay-based entry will change where the view comes from, not this contract.

## 9. Multi-Agent Rules

The launcher contract must preserve the existing attach model:

1. all children on one runtime share one `runtime_session_id`;
2. each child gets its own `attach_id`;
3. `APPFS_AGENT_ROLE` is descriptive only and may repeat;
4. no shared attached-agent registry file is required in this phase.

The first implementation may launch just one child agent, but the contract must not block later expansion to many children.

## 10. Overlay Compatibility

This launcher contract is intentionally designed so future overlay-backed entry can reuse it.

Future shape:

1. a user chooses an existing host directory;
2. that directory becomes overlay base input to AppFS;
3. AppFS exposes a separate mounted runtime view;
4. the launcher starts `appfs-agent` inside the runtime view.

Cross-platform baseline:

1. Windows does not need same-path in-place mount behavior for this phase;
2. Linux and macOS may support tighter same-path workflows later;
3. the stable product contract stays "existing directory as input, AppFS runtime view as execution path".

## 11. Recommended First Implementation

The first implementation should stay narrow.

Recommended shape:

1. support one launcher-owned AppFS runtime;
2. support one child `appfs-agent` process first;
3. use explicit attach env injection only;
4. default child cwd to `<mount_root>/workspace`;
5. keep heuristic detection as fallback only for non-launcher flows;
6. add integration coverage for the explicit startup path before adding richer UX.

Command surface should stay flexible for now. Valid first implementations include:

1. a dedicated wrapper command;
2. an `agentfs appfs agent run ...` entrypoint;
3. an `agentfs appfs up --launch-agent ...` mode if lifecycle semantics remain explicit.

The important contract is the startup sequence and child environment, not the final command name.

Current prototype surface in `appfs-platform`:

1. `agentfs appfs launch <id-or-path> <mountpoint> --agent-bin <path-to-appfs-agent> [--workspace <relative-path>] [--attach-id <id>] [--attach-role <role>] -- <agent-args...>`
2. the launcher starts `agentfs appfs up` as a child process, waits for the runtime manifest, then launches one child agent with explicit `APPFS_ATTACH_*` env
3. this prototype is intentionally single-child only

## 12. Proposed Next Checkpoint

The next integration checkpoint after `IC-2` should validate explicit launcher startup.

Proposed `IC-3` goals:

1. one command starts AppFS and one child `appfs-agent`;
2. the launcher waits for runtime manifest readiness before child launch;
3. the child reports `appfs.attach_source = env`;
4. the child reports the same `runtime_session_id` found in the runtime manifest;
5. the child `cwd` is inside the mounted AppFS workspace;
6. no manual shell injection of `APPFS_ATTACH_*` is required from the operator.

`IC-3` should still avoid:

1. principal-aware visibility;
2. shared/private app policy;
3. same-path overlay startup;
4. multi-child orchestration if a single-child baseline has not yet stabilized.

## 13. Open Decisions

These choices still need to be made before implementation:

1. which launcher command surface becomes the first supported UX;
2. whether the first launcher version should live in standalone `appfs` CLI or as a small wrapper first;
3. whether the first child workspace override should be supported immediately or deferred until after the default workspace path is stable;
4. when to promote explicit launcher startup from optional integration flow into the default happy path.

## 14. Recommendation

The next implementation step should be:

1. keep `IC-0`, `IC-1`, and `IC-2` green;
2. prototype one explicit launcher-driven startup path;
3. automate it as `IC-3`;
4. only then continue to overlay-backed existing-directory entry and richer AppFS-aware UX.
