# Principal Registry Refresh and Multi-Agent Awareness Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep AppFS as the sole source of principal truth while making newly created principals visible to all active agents and the dashboard without mutating system prompts or duplicating registry ownership.

**Architecture:** AppFS owns `principals.registry.json` and `apps.registry.json`. `appfs-agent` loads a snapshot of the principal registry plus a revision marker, then refreshes that snapshot at each model boundary. The dashboard remains a read-only projection over the same files and process state. Registry changes are durable and replayable, not single-consumer chat events.

**Tech Stack:** Rust (`appfs-agent` runtime and CLI), TypeScript (`dashboard/server`), Fastify, JSON/JSONL, file watcher, existing AppFS registry files.

---

## Constraints

- Do not introduce `/principal use`.
- Do not create a second principal registry in dashboard.
- Do not mutate the system prompt in place mid-session.
- Do not use one-shot events as the only delivery path for principal registry changes.
- Keep the current AppFS principal/app model intact.

## Task 1: Add Principal Registry Revision Plumbing

**Files:**
- Modify: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Test: `appfs-agent/rust/crates/runtime/src/appfs.rs` tests

**Step 1: Write the failing tests**

Add focused tests that prove:
- a principal registry change produces a new revision value;
- `known_principals` refreshes when the registry revision changes;
- the runtime keeps the same current principal but refreshes the roster snapshot.

**Step 2: Run the tests and verify they fail**

Run:
```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime principal_registry -- --nocapture
```

Expected: the new revision/refresh tests fail or do not compile yet.

**Step 3: Implement the revision snapshot**

Add a principal registry revision field to the AppFS environment, derived from the registry file content or a stable file signature.

Use that revision to decide whether the cached `known_principals` snapshot is stale.

**Step 4: Re-run the tests**

Run:
```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime principal_registry -- --nocapture
```

Expected: pass.

**Step 5: Commit**

Commit the runtime revision plumbing once the tests are green.

---

## Task 2: Inject Principal Registry Refresh Into Model Boundaries

**Files:**
- Modify: `appfs-agent/rust/crates/runtime/src/conversation.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Test: existing runtime boundary tests

**Step 1: Write the failing test**

Add a test proving that when the principal registry revision changes, the next model boundary receives a short reminder or refresh block, rather than a permanent system prompt rewrite.

The reminder should be concise, e.g.:
- registry updated
- new principals available
- refresh current roster

**Step 2: Run the test and verify it fails**

Run:
```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime model_boundary -- --nocapture
```

**Step 3: Implement the refresh injection**

At model boundary time:
- compare the cached registry revision with the current one;
- if changed, inject a small reminder into the next turn input;
- update the cached revision after injection.

Do not rewrite the base system prompt.

**Step 4: Re-run the test**

Run:
```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime model_boundary -- --nocapture
```

Expected: pass.

**Step 5: Commit**

Commit the boundary refresh logic.

---

## Task 3: Add a Read-Only Principal Projection to Dashboard

**Files:**
- Create: `dashboard/server/src/routes/principals.ts`
- Modify: `dashboard/server/src/index.ts`
- Modify: `dashboard/server/src/types.ts`
- Optional UI: `dashboard/src/components/AgentSidebar.tsx` or `dashboard/src/components/InfoPanel.tsx`

**Step 1: Write the failing API test**

Add a test or manual route check that reads `/_appfs/principals.registry.json` and returns the parsed roster.

The route should be read-only and should not mutate any state.

**Step 2: Run the test and verify it fails**

Run:
```powershell
npm run -w dashboard build
```

Expected: the new route is missing until implemented.

**Step 3: Implement the projection route**

Add a `/api/principals` route that reads the AppFS principal registry file from the mounted project root.

Make the dashboard treat it as a projection only:
- no writes;
- no local source of truth;
- optional file-watcher refresh is allowed.

**Step 4: Wire the UI**

If needed, show the live principal roster in the dashboard alongside the active agent list, but keep the distinction clear:
- principals = project identity roster;
- agents = live sessions/processes.

**Step 5: Re-run the build**

Run:
```powershell
npm run -w dashboard build
```

Expected: pass.

**Step 6: Commit**

Commit the dashboard projection route.

---

## Task 4: Optional Read-Only Principal Status Helper

**Files:**
- Modify: `appfs-agent/rust/crates/runtime/src/appfs.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`
- Test: runtime prompt/status tests

**Step 1: Decide the helper surface**

Choose one minimal helper:
- `appfs.list_principals`
- `appfs.status`

This helper is only for on-demand inspection and should not replace boundary refreshes.

**Step 2: Add a failing test**

Verify the helper returns the current roster snapshot without mutating registry state.

**Step 3: Implement the helper**

Expose the roster as a read-only model-facing action or status output.

**Step 4: Re-run the test**

Run:
```powershell
cargo test --manifest-path appfs-agent\rust\Cargo.toml -p runtime principal_status -- --nocapture
```

Expected: pass.

**Step 5: Commit**

Commit only if the helper proves useful in practice.

---

## Verification Checklist

- A newly created principal appears in `principals.registry.json`.
- Existing agents learn about the change on their next model boundary.
- The dashboard reflects the same registry snapshot.
- No second principal registry exists in Node.
- No system prompt rewrite is needed for roster changes.
- One agent consuming a change does not starve other agents.

## Rollout Order

1. Principal registry revision plumbing.
2. Boundary refresh injection.
3. Dashboard read-only projection.
4. Optional helper tool only if needed.

## Success Criteria

- AppFS remains the only authority for principals.
- Agents stay aware of new principals without prompt mutation.
- Dashboard shows the same truth as AppFS, but does not own it.
- Registry changes are durable, replayable, and visible to all active agents.

