# Integration Workspace

This directory is reserved for scenarios that span both `appfs` and `appfs-agent`.

## Start Here

1. [AppFS Platform Unified Roadmap v0.1](./APPFS-platform-roadmap-v0.1.md)
2. [AppFS x appfs-agent Attach Contract v1.1](./APPFS-appfs-agent-attach-contract-v1.1.md)
3. [AppFS Joint Startup / Launcher Contract v0.1](./APPFS-joint-startup-launcher-contract-v0.1.md)
4. `integration/scripts/test-windows-appfs-agent-smoke.ps1`
5. `integration/scripts/test-windows-appfs-agent-http-demo.ps1`
6. `integration/scripts/test-windows-appfs-agent-multi-attach.ps1`

## Intended Contents

- `fixtures/`: reusable mounted-tree examples and golden data
- `scripts/`: local bring-up helpers for mounts, bridges, and agent sessions
- `tests/`: end-to-end validation that exercises both layers together

## First Recommended Scenarios

1. Mount AppFS and verify the runtime manifest is published at `/.well-known/appfs/runtime.json`.
2. Verify `appfs-agent` can resolve AppFS attach state and surface it in `/status`.
3. Trigger `*.act` writes from the agent runtime and verify resulting event streams.

The goal is to keep shared integration assets here, not to duplicate each subproject's own unit or contract tests.

## Current Smoke Automation

Current contract mapping:

| Script | Checkpoint | Contract clauses |
|---|---|---|
| `integration/scripts/test-windows-appfs-agent-smoke.ps1` | `IC-0` | `C0`, `C1`, `C2`, `C3` |
| `integration/scripts/test-windows-appfs-agent-http-demo.ps1` | `IC-1` | `C0`, `C1`, `C4` |
| `integration/scripts/test-windows-appfs-agent-multi-attach.ps1` | `IC-2` | `C0`, `C1`, `C2`, `C3`, `C5` |

For the first Windows integration checkpoint, use:

- `integration/scripts/test-windows-appfs-agent-smoke.ps1`
- `integration/scripts/test-windows-appfs-agent-http-demo.ps1`

What it validates:

1. initialize an AppFS database
2. mount AppFS on WinFsp
3. confirm AppFS publishes `/.well-known/appfs/runtime.json`
4. create a mounted workspace and `hello.txt`
5. run `appfs-agent` (`claw status`) with the mounted workspace as `cwd`
6. verify `/status` shows manifest attach metadata
7. optionally run one prompt against the mounted workspace when `-RunPrompt` is supplied

This is the current implementation of `IC-0` from the integration contract.

Example:

```powershell
./integration/scripts/test-windows-appfs-agent-smoke.ps1
```

Optional real-provider prompt:

```powershell
$env:ANTHROPIC_BASE_URL="https://open.bigmodel.cn/api/anthropic"
$env:ANTHROPIC_API_KEY="..."
./integration/scripts/test-windows-appfs-agent-smoke.ps1 -RunPrompt
```

The repository also includes an opt-in workflow at `.github/workflows/integration-smoke-windows.yml`.
It is designed for a self-hosted Windows runner with WinFsp installed, because GitHub-hosted Windows runners do not provide the mount dependency by default.

## HTTP Demo Integration

For the second Windows checkpoint, use the HTTP demo bridge plus a real `appfs-agent` prompt:

```powershell
$env:ANTHROPIC_BASE_URL="https://open.bigmodel.cn/api/anthropic"
$env:ANTHROPIC_API_KEY="..."
./integration/scripts/test-windows-appfs-agent-http-demo.ps1
```

What it validates:

1. starts the AppFS HTTP demo bridge
2. mounts AppFS on WinFsp
3. confirms AppFS publishes `/.well-known/appfs/runtime.json`
4. registers the demo `aiim` app through `/_appfs/register_app.act`
5. verifies `chats/chat-001/messages.res.jsonl` is readable from the mounted app tree
6. runs `appfs-agent` with `bash` as the only allowed tool
7. has the agent submit one `contacts/zhangsan/send_message.act` request and confirm the token appears in `_stream/events.evt.jsonl`

This is the current implementation of `IC-1` from the integration contract.

The corresponding workflow is `.github/workflows/integration-http-demo-windows.yml`.
It stays `workflow_dispatch` only on purpose:

- it requires provider credentials
- it depends on a self-hosted Windows runner with WinFsp
- it incurs external model cost and network variability

If we later want broader automation, the next sensible step is a scheduled or label-triggered version of the same workflow, still backed by repository secrets rather than hard-coded credentials.

## Planned IC-2 Automation

The next integration checkpoint is `IC-2`.

What it should validate:

1. start one AppFS mount
2. start at least two `appfs-agent` processes against that same mount
3. inject the same runtime manifest and mount metadata into both agents
4. inject distinct `APPFS_ATTACH_ID` values
5. verify both agents report the same `runtime_session_id`
6. verify both agents report different `attach_id` values
7. verify both agents resolve AppFS attach from `env`

What it should not validate yet:

1. shared/private app visibility
2. per-agent app-side account separation
3. principal-aware path routing
4. launcher-managed joint startup

That keeps `IC-2` focused on Phase 1 attach semantics rather than future identity policy.

Example:

```powershell
./integration/scripts/test-windows-appfs-agent-multi-attach.ps1
```

The corresponding manual workflow is `.github/workflows/integration-multi-attach-windows.yml`.
It is `workflow_dispatch` only for now so we can validate the scenario on the self-hosted WinFsp runner before promoting it into required PR CI.

## Next Design Focus

With `IC-2` in place, the next design step is explicit launcher-driven startup.

The current direction is:

1. AppFS startup should wait for mount and runtime manifest readiness;
2. a launcher should start `appfs-agent` inside an AppFS-backed workspace;
3. the launcher should inject `APPFS_ATTACH_*` explicitly instead of relying on heuristic discovery;
4. future overlay-backed "start from existing directory" flows should reuse the same launch contract.

Current prototype command:

```powershell
agentfs appfs launch <id-or-path> <mountpoint> --agent-bin <path-to-appfs-agent> -- status --output-format json
```

Tracked in:

1. [AppFS Joint Startup / Launcher Contract v0.1](./APPFS-joint-startup-launcher-contract-v0.1.md)
