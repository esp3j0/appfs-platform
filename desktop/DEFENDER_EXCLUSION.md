# Windows Defender Exclusion for AppFS Desktop

## Why

On Windows, `agentfs.exe` mounts the AppFS tree as a **WinFsp local disk
volume**. Windows Defender's real-time protection (`MsMpEng.exe`) scans files on
that volume. The supervisor rewrites control-plane files on principal lifecycle
changes — e.g. `_appfs/principals.registry.json` is rewritten on every
attach / detach / create / delete / sweep. Each rewrite triggers Defender to
**scan** the file; because the file lives on a user-mode (WinFsp) volume,
servicing Defender's read requires `agentfs.exe` to do user-mode I/O, which
Defender's minifilter in turn re-intercepts — producing a self-sustaining
**read-loop** that burns ~3–8% CPU and can crash `MsMpEng.exe` (Windows Defender
Event ID 5008, `DEADLOCKS_DETECTED`).

A **host-folder exclusion** (e.g. excluding `C:\Users\...\rep`) does **not** fix
this, because Defender accesses the file through the WinFsp **volume device
path** (`\Device\Volume{...}\_appfs\...`), which is not matched by host-path
exclusions. A **process exclusion** is required: Defender will not scan any file
read or written by the excluded process.

## Fix A (required) — process exclusion

The NSIS installer (`build/installer.nsh`) applies the exclusions automatically
on install and removes them on uninstall:

- `agentfs.exe`
- `claw.exe`

To apply manually on a dev / CI machine, run as Administrator:

```powershell
powershell -ExecutionPolicy Bypass -File desktop\scripts\add-defender-exclusion.ps1
```

Verify:

```powershell
(Get-MpPreference).ExclusionProcess   # should list agentfs.exe and claw.exe
```

## Fix B (opt-in, experimental) — mount as a network volume

Set `APPFS_WINFSP_NETWORK_VOLUME=1` for the `agentfs` process to mount the
volume via the **WinFsp network provider** (`WinFsp.Net`) instead of the local
disk provider (`WinFsp.Disk`). Defender does not real-time-scan network volumes
by default, so this sidesteps the loop without any exclusion.

This is gated behind an env var because a network volume can change mountpoint
access semantics. After enabling, verify:

1. the mountpoint is still reachable,
2. agents can still attach (`claw status`),
3. `procmon` shows `MsMpEng.exe` no longer reading `_appfs/principals.registry.json`.

If anything breaks, unset `APPFS_WINFSP_NETWORK_VOLUME` to revert to local-disk
behavior (and rely on Fix A).

A and B are complementary: keep A on everywhere; enable B to additionally avoid
the scan trigger at the source.
