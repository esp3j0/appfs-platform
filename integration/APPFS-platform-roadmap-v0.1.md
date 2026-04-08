# AppFS Platform Unified Roadmap v0.1

**Status:** Working roadmap  
**Scope:** `appfs` + `appfs-agent` + `appfs-platform/integration`  
**Updated:** 2026-04-07

## 1. Why This Roadmap Exists

`appfs` and `appfs-agent` already have their own architecture and implementation history, but the current product direction is no longer just "make each repo advance independently".

The real integration surface is now:

1. AppFS publishes a stable runtime and app contract.
2. `appfs-agent` attaches to that runtime and operates against mounted app trees.
3. `appfs-platform` carries the cross-project contract, smoke automation, and end-to-end checkpoints.

This roadmap exists to make the next sequence explicit, so we do not jump into higher-level identity or product semantics before the attach and launch path are stable.

## 2. Current Baseline

The current baseline is:

1. `appfs` publishes `/.well-known/appfs/runtime.json`.
2. `appfs-agent` resolves AppFS attach in the order `env > manifest > heuristic`.
3. multi-agent attach is supported at the contract level through one shared `runtime_session_id` plus distinct per-agent `attach_id` values.
4. `/status` exposes AppFS attach metadata in both text and JSON output.
5. `IC-0`, `IC-1`, and `IC-2` now exist as concrete Windows integration checkpoints.

Primary source documents:

1. [AppFS x appfs-agent Attach Contract v1.1](./APPFS-appfs-agent-attach-contract-v1.1.md)
2. [AppFS Joint Startup / Launcher Contract v0.1](./APPFS-joint-startup-launcher-contract-v0.1.md)
3. [AppFS Principal Visibility And Agent Identity Design](../appfs/docs/plans/2026-04-07-appfs-principal-visibility-and-agent-identity.md)

## 3. Guiding Principle

The next work should move in this order:

1. stabilize attach;
2. stabilize multi-agent attach;
3. stabilize launcher-driven startup;
4. establish an overlay-backed existing-directory entry path;
5. improve AppFS-aware agent UX;
6. only then add principal-aware visibility and app identity semantics.

This keeps the base platform debuggable. If we mix attach, launch, identity, and connector session isolation too early, regressions will be difficult to localize.

## 4. Workstreams

### R0. Stable Attach Contract

**Status:** Complete enough for current baseline

Delivered:

1. manifest-based attach contract
2. attach env contract
3. `runtime_session_id` / `attach_id` separation
4. `/status` attach visibility
5. `IC-0` and `IC-1` checkpoints

Exit criteria:

1. keep `IC-0` and `IC-1` green on every integration change
2. keep heuristic attach only as compatibility fallback

### R1. Multi-Agent Baseline Automation

**Status:** Landed as the current multi-agent baseline

Goal:

1. automate `IC-2`, not just hand-test it

Deliverables:

1. an integration scenario that launches at least two `appfs-agent` processes against one AppFS mount
2. assertions that both agents share one `runtime_session_id`
3. assertions that each agent keeps a distinct `attach_id`
4. optional self-hosted Windows CI coverage for the same scenario

Why this is next:

1. the contract already says multi-agent attach is first-class
2. it replaced what used to be mostly manual proof
3. it is the highest-value regression guard before deeper runtime or UX changes

### R2. Launcher-Driven Startup

**Status:** Prototype implemented in the integration workspace; stabilization is the next target

Goal:

1. make the primary attach path explicit at process launch time instead of inferred from mounted directory structure

Target shape:

1. AppFS bring-up waits until mount and runtime manifest are ready
2. a launcher starts `appfs-agent`
3. the launcher injects `APPFS_ATTACH_*` environment variables
4. heuristic directory detection remains fallback-only

Candidate surfaces:

1. `agentfs appfs up --launch-agent ...`
2. `agentfs appfs agent run ...`
3. a small standalone launcher wrapper if CLI integration should remain minimal at first

Exit criteria:

1. one documented and supported launch path for "AppFS + appfs-agent together"
2. attach works without relying on control-plane directory naming
3. the launched agent runs inside an AppFS-backed workspace rather than relying on a raw host directory
4. `IC-3` stays green on the self-hosted Windows launcher workflow

### R3. Overlay-Backed Existing Directory Entry

**Status:** Future work after launcher-driven startup

Goal:

1. let a user start `appfs-agent` from a known host directory while still running on an AppFS-backed workspace
2. use overlay as the compatibility bridge between an existing host directory and the AppFS runtime view

Target shape:

1. a host directory may be used as AppFS overlay `--base`
2. AppFS mounts a separate runtime view path by default
3. `appfs-agent` starts with its `cwd` inside that mounted AppFS view, not inside the raw host directory
4. the user experience should feel like "start from this directory", even if the runtime view lives at a generated mount path

Cross-platform note:

1. Windows should not require in-place mounting over an already existing directory path, because the current WinFsp path expects a non-existent mountpoint
2. Linux and macOS may later support tighter same-path workflows, but that is not the baseline contract for this phase
3. the stable cross-platform target is "existing directory as overlay base, separate AppFS runtime view, agent launched inside the view"

Non-goals for this phase:

1. do not require true same-path mount parity across all platforms
2. do not mix this phase with principal-aware visibility or app identity policy
3. do not replace the current attach contract with overlay-specific rules

### R4. AppFS-Aware Agent UX

**Status:** Planned after launcher-driven startup

Goal:

1. make `appfs-agent` clearly stronger when running on AppFS, while keeping it usable as a generic agent outside AppFS

Likely scope:

1. richer `/status` and context display for mounted apps and active app roots
2. easier discovery of readable `*.res.jsonl`, writable `*.act`, and `_stream/events.evt.jsonl`
3. launcher-injected context or startup hints for mounted apps
4. possible future AppFS-backed workspace boundary display in `/sandbox` or related status output

Non-goal for this phase:

1. do not turn `appfs-agent` into an AppFS-only agent

### R5. Principal And Visibility Model

**Status:** Future work, intentionally deferred

Tracked in:

1. [AppFS Principal Visibility And Agent Identity Design](../appfs/docs/plans/2026-04-07-appfs-principal-visibility-and-agent-identity.md)

Goal:

1. support shared and private app views in AppFS
2. let different agents operate under different app-side identities
3. keep visibility policy in AppFS rather than in the agent client

This should only begin after:

1. attach correctness is stable
2. `IC-2` is automated
3. registered app loops remain green under multi-agent scenarios

## 5. Explicitly Deferred

The following are not current mainline goals:

1. replacing OS-level sandboxing with AppFS security semantics
2. full principal-aware app routing
3. connector-side account/profile isolation across principals
4. broad brand or narrative migration before runtime behavior is ready
5. large-scale parity chasing with upstream `claw` that is not directly justified by AppFS integration needs
6. requiring same-path in-place mount behavior on every platform before AppFS-backed workspace mode can ship

## 6. Suggested Near-Term Milestones

### M1. Multi-Agent CI Baseline

1. add `IC-2` automation
2. run it locally first
3. promote it into self-hosted Windows CI once stable

**Current note:** The baseline now exists; the ongoing task is to keep it green.

### M2. Explicit Joint Startup

1. stabilize the first supported launcher entrypoint
2. keep manifest path, mount root, runtime session, and attach id injection explicit
3. document and keep the supported startup flow green in `integration/`

### M3. Overlay-Backed Workspace Entry

1. choose the first supported "start from existing directory" flow
2. use overlay `--base` plus a separate AppFS runtime mount as the primary cross-platform model
3. document clearly that Windows does not need same-path mount semantics for this milestone

### M4. AppFS-Aware Quality-Of-Life Pass

1. expose mounted app context better in `appfs-agent`
2. improve operator visibility for current app, control plane, and events
3. make AppFS-backed runs feel intentional rather than accidental

## 7. Repository Ownership

This roadmap is cross-project, but code ownership still follows repository boundaries:

1. `appfs-platform/integration/`
   contract docs, smoke scripts, end-to-end checkpoints, roadmap
2. standalone `appfs`
   runtime manifest, launcher behavior, mount/runtime semantics, visibility policy
3. standalone `appfs-agent`
   attach resolution, status output, AppFS-aware UX, optional launcher consumption

## 8. Recommended Immediate Next Step

If we pick one concrete next task from this roadmap, it should be:

1. stabilize `IC-3` and the first launcher-driven startup path for AppFS-backed agent execution

That is the shortest path to making AppFS-backed agent execution feel explicit and productized instead of manually assembled.
