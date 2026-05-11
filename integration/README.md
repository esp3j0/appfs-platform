# Integration Workspace

This directory is reserved for scenarios that span both `appfs` and `appfs-agent`.

## Start Here

1. [AppFS Platform Unified Roadmap v0.1](./APPFS-platform-roadmap-v0.1.md)
2. [AppFS x appfs-agent Attach Contract v1.1](./APPFS-appfs-agent-attach-contract-v1.1.md)
3. [AppFS Joint Startup / Launcher Contract v0.1](./APPFS-joint-startup-launcher-contract-v0.1.md)
4. `integration/scripts/test-windows-appfs-agent-smoke.ps1`
5. `integration/scripts/test-windows-appfs-agent-http-demo.ps1`
6. `integration/scripts/test-windows-appfs-agent-multi-attach.ps1`
7. `integration/scripts/test-windows-appfs-agent-launcher.ps1`
8. `integration/scripts/test-unix-appfs-agent-smoke.sh`
9. `integration/scripts/test-windows-appfs-tinode-multi-agent-smoke.ps1`

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
| `integration/scripts/test-windows-appfs-agent-launcher.ps1` | `IC-3` | `C0`, `C1`, `C2`, `C3`, `C6` |

## Unix Local Smoke

For Linux and macOS, use:

- `integration/scripts/test-unix-appfs-agent-smoke.sh`

It validates the same attached-workspace baseline as the Windows `IC-0` smoke:

1. initialize an AppFS database
2. start `appfs up` in managed mode
3. confirm AppFS publishes `/.well-known/appfs/runtime.json`
4. create a mounted workspace and `hello.txt`
5. run `appfs-agent` (`claw status`) with the mounted workspace as `cwd`
6. verify `/status` shows manifest attach metadata
7. optionally run one prompt against the mounted workspace when `--run-prompt` is supplied

Backend mapping:

- Linux: `--backend fuse`
- macOS: `--backend nfs`

Example:

```bash
bash ./integration/scripts/test-unix-appfs-agent-smoke.sh
```

Optional real-provider prompt:

```bash
export ANTHROPIC_BASE_URL="https://open.bigmodel.cn/api/anthropic"
export ANTHROPIC_API_KEY="..."
bash ./integration/scripts/test-unix-appfs-agent-smoke.sh --run-prompt
```

Notes:

- Linux requires `/dev/fuse` plus `fusermount3` or `fusermount`.
- macOS uses the existing localhost NFS backend and requires `mount_nfs`.
- This helper is local-only for now; it is not wired into GitHub Actions yet because hosted runners do not reliably provide the required FUSE/NFS mount privileges.

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

## Tinode Multi-Agent Smoke

For the current AppFS + Tinode multi-agent identity and messaging flow, use:

```powershell
./integration/scripts/test-windows-appfs-tinode-multi-agent-smoke.ps1
```

What it validates:

1. start AppFS in Tinode-only compose mode;
2. trigger principal attach through `claw status` for `default` and `code-implementer`;
3. verify `private/default/tinode` and `private/code-implementer/tinode` are materialized by attach, not by AppFS startup alone;
4. verify Tinode credential warmup and direct messages between the two principals;
5. verify both inboxes receive the opposite principal's message.

This smoke is local-only for now and is not yet wired into GitHub Actions.

### Manual bring-up

If you want to reproduce the same flow by hand, use this sequence:

1. Reset the remote Tinode test server if you want a clean slate:

```powershell
ssh ubuntu@101.34.216.193 "/home/ubuntu/reset-appfs-tinode.sh --skip-backup"
```

2. Start AppFS with the Tinode-only compose file:

```powershell
cd C:\Users\esp3j\rep\appfs-platform
$env:APPFS_TINODE_ENDPOINT="http://101.34.216.193:6060"
$env:APPFS_TINODE_LOGIN_PREFIX="appfsmanual$(Get-Date -Format yyyyMMddHHmmss)"
$env:APPFS_TINODE_CREDENTIAL_POLICY="auto-create"
cargo run --manifest-path appfs\cli\Cargo.toml --target-dir C:\tmp\appfs-local-target -- appfs compose up -f appfs\appfs-compose.tinode.local.yaml
```

3. Start the default agent:

```powershell
cd C:\mnt\appfs-compose-tinode
cargo run --manifest-path C:\Users\esp3j\rep\appfs-platform\appfs-agent\rust\Cargo.toml --target-dir C:\tmp\appfs-agent-local-target -p rusty-claude-cli -- --dangerously-skip-permissions --appfs-idle-wake --running-input
```

4. Start the second agent as `code-implementer`:

```powershell
cd C:\mnt\appfs-compose-tinode
$env:APPFS_PRINCIPAL_ID="code-implementer"
cargo run --manifest-path C:\Users\esp3j\rep\appfs-platform\appfs-agent\rust\Cargo.toml --target-dir C:\tmp\appfs-agent-local-target -p rusty-claude-cli -- --dangerously-skip-permissions --appfs-idle-wake --running-input
```

5. Send a message from `default` to `code-implementer` by appending one JSON line to `private/default/tinode/contacts/send_message.act`, then check `private/code-implementer/tinode/_stream/events.evt.jsonl` and `private/code-implementer/tinode/inbox/recent.res.jsonl`.

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

## Launcher Integration

The next launcher checkpoint is `IC-3`.

What it validates:

1. one supported command starts AppFS and one `appfs-agent` child together
2. the launcher waits for `/.well-known/appfs/runtime.json` before child launch
3. the launcher injects `APPFS_ATTACH_*` explicitly
4. the child `cwd` lands inside `<mount_root>/workspace`
5. the child reports `appfs.attach_source = env`
6. the child reports no AppFS attach warnings

Example:

```powershell
./integration/scripts/test-windows-appfs-agent-launcher.ps1
```

Optional local reuse of already-built binaries:

```powershell
./integration/scripts/test-windows-appfs-agent-launcher.ps1 -SkipBuild
```

The corresponding manual workflow is `.github/workflows/integration-launcher-windows.yml`.
It stays `workflow_dispatch` only for now because it depends on a self-hosted Windows runner with WinFsp and a working Windows SDK / libclang environment for fresh builds.

Tracked in:

1. [AppFS Joint Startup / Launcher Contract v0.1](./APPFS-joint-startup-launcher-contract-v0.1.md)
