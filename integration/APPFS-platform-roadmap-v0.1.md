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
5. `IC-0` and `IC-1` Windows integration checkpoints are automated and green.

Primary source documents:

1. [AppFS x appfs-agent Attach Contract v1.1](./APPFS-appfs-agent-attach-contract-v1.1.md)
2. [AppFS Principal Visibility And Agent Identity Design](../appfs/docs/plans/2026-04-07-appfs-principal-visibility-and-agent-identity.md)

## 3. Guiding Principle

The next work should move in this order:

1. stabilize attach;
2. stabilize multi-agent attach;
3. stabilize launcher-driven startup;
4. improve AppFS-aware agent UX;
5. only then add principal-aware visibility and app identity semantics.

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

**Status:** Next priority

Goal:

1. automate `IC-2`, not just hand-test it

Deliverables:

1. an integration scenario that launches at least two `appfs-agent` processes against one AppFS mount
2. assertions that both agents share one `runtime_session_id`
3. assertions that each agent keeps a distinct `attach_id`
4. optional self-hosted Windows CI coverage for the same scenario

Why this is next:

1. the contract already says multi-agent attach is first-class
2. current proof is mostly manual
3. this is the highest-value regression guard before deeper runtime or UX changes

### R2. Launcher-Driven Startup

**Status:** Planned after `IC-2`

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

### R3. AppFS-Aware Agent UX

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

### R4. Principal And Visibility Model

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

## 6. Suggested Near-Term Milestones

### M1. Multi-Agent CI Baseline

1. add `IC-2` automation
2. run it locally first
3. promote it into self-hosted Windows CI once stable

### M2. Explicit Joint Startup

1. choose the first supported launcher entrypoint
2. inject manifest path, mount root, runtime session, and attach id explicitly
3. document the supported startup flow in `integration/`

### M3. AppFS-Aware Quality-Of-Life Pass

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

1. implement `IC-2` multi-agent attach automation in `appfs-platform/integration`

That is the shortest path to turning the current contract from "validated by hand" into "protected by automation".
