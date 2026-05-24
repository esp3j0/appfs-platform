# WebUI-Controlled AppFS Project Runtime Plan

## Goal

Let the existing dashboard control AppFS directly, before we wrap everything into Electron.

This turns the dashboard into the project entry point:

1. user selects a project folder;
2. dashboard starts or stops `appfs compose up`;
3. dashboard groups agents by project;
4. the same control plane can later be embedded in Electron without redoing the architecture.

## Current State

The workspace already has the key building blocks:

1. `appfs compose up` is the authoritative runtime/bootstrap path.
2. Dashboard already manages `appfs-agent` processes.
3. Dashboard already tracks `cwd`, `appfsMountRoot`, `principalId`, `sessionId`, and `controlMode`.
4. Agent registry already distinguishes managed vs external agents.

Relevant files:

- [appfs/cli/src/cmd/appfs.rs](C:/Users/esp3j/rep/appfs-platform/appfs/cli/src/cmd/appfs.rs)
- [appfs/docs/plans/2026-04-22-appfs-compose-up-design.md](C:/Users/esp3j/rep/appfs-platform/appfs/docs/plans/2026-04-22-appfs-compose-up-design.md)
- [dashboard/server/src/process-manager.ts](C:/Users/esp3j/rep/appfs-platform/dashboard/server/src/process-manager.ts)
- [dashboard/server/src/agent-registry.ts](C:/Users/esp3j/rep/appfs-platform/dashboard/server/src/agent-registry.ts)

## Key Decision

Use a sidecar directory inside the project:

- project root: the real user workspace
- `projectRoot/.appfs`: AppFS mountpoint only
- `projectRoot/.appfs-compose.yaml`: compose source of truth

This keeps project files and AppFS state separate.

## Path Model

### Project paths

- `projectRoot`: the folder the user selects in the dashboard
- `composeFilePath`: `projectRoot/.appfs-compose.yaml`
- `mountRoot`: `projectRoot/.appfs`
- `sessionRoot`: existing `.claw` session storage, unchanged

### Resolution rules

1. Project source files are always resolved from `projectRoot`.
2. AppFS tree paths are always resolved from `mountRoot`.
3. Dashboard file-scanning for project content must ignore `.appfs` and `.claw`.
4. Any AppFS tree inspection must add one extra `.appfs` layer.

Examples:

- project file: `projectRoot/src/main.ts`
- AppFS file: `projectRoot/.appfs/private/default/tinode/inbox/recent.res.jsonl`

## Why Not The Other Options

### Not overlay first

Overlay is useful later, but it is not the first step.

Reason:

1. overlay mixes project content semantics with AppFS runtime semantics;
2. we already have separate `cwd` and `appfsMountRoot` in the dashboard;
3. the project-control problem is mostly lifecycle and grouping, not filesystem virtualization.

### Not temp + copy

This is only a workaround.

Reason:

1. it breaks live project editing;
2. it creates drift between the real project and copied workspace;
3. it adds unnecessary data movement and cleanup complexity.

## Important Boundary

`projectRoot/.appfs` is mountpoint-only.

It must not contain the compose file or other pre-existing project data, because the runtime mountpoint must be empty or absent before startup on WinFsp.

If the project already has a non-empty `.appfs` directory, dashboard should treat that as a conflict and ask the user to resolve it.

## Runtime Model

### Project runtime

Each project gets one runtime record:

```ts
interface ProjectRuntimeInfo {
  projectId: string;
  projectRoot: string;
  composeFilePath: string;
  mountRoot: string;
  status: 'stopped' | 'starting' | 'running' | 'error';
  managedAgentSessionIds: string[];
}
```

### Agent model

Agent records should gain project linkage:

```ts
interface AgentInfo {
  projectId: string;
  projectRoot: string;
  principalId: string;
  sessionId: string;
  controlMode: 'managed' | 'external';
}
```

The dashboard should group agents by `projectId`, then by `principalId`.

## UI Plan

### Left Sidebar

Replace the flat agent list with:

1. project group header
2. project runtime status
3. agents under that project
4. per-agent principal/session/control mode

### Project Actions

Each project should have:

1. `Start AppFS`
2. `Stop AppFS`
3. `Spawn Agent`
4. `Open Folder`

### Hidden folders

The UI should hide `.appfs` and `.claw` from ordinary project file browsing.

Only the AppFS control pane should show `.appfs` as a runtime root.

## Backend Plan

### New responsibilities

1. project discovery
2. project runtime lifecycle
3. project-to-agent grouping
4. compose file resolution
5. mountpoint hygiene checks

### Suggested API

- `GET /api/projects`
- `POST /api/projects/open`
- `POST /api/projects/:projectId/start`
- `POST /api/projects/:projectId/stop`
- `GET /api/projects/:projectId/status`

### Existing process manager

`process-manager.ts` should keep owning managed agent spawn/stop/prompt behavior.

It should accept a project-scoped spawn config so each agent knows:

1. `cwd` = project root
2. `appfsMountRoot` = `projectRoot/.appfs`
3. `APPFS_PRINCIPAL_ID` = selected principal

## Lifecycle

### Start

1. user picks a project folder;
2. dashboard finds or creates `projectRoot/.appfs-compose.yaml`;
3. dashboard validates `projectRoot/.appfs` is safe to mount;
4. dashboard runs `appfs compose up -f <composeFilePath>`;
5. dashboard refreshes project/agent registry;
6. dashboard may then spawn managed agents for that project.

### Stop

1. stop managed agents for that project;
2. stop AppFS runtime;
3. unmount `projectRoot/.appfs`;
4. leave the project files intact.

## Edge Cases

1. Existing `.appfs` directory with files.
2. Existing stale mountpoint.
3. Windows parent directory exists but mountpoint must be removed before WinFsp mount.
4. Multiple projects with the same folder name.
5. Mixed external and managed agents in one project.
6. Paths with spaces or non-ASCII characters.

## Implementation Order

### P0

Add project runtime registry and compose file discovery.

### P1

Add project start/stop endpoints and mountpoint validation.

### P2

Group dashboard agents by project.

### P3

Hide `.appfs` from normal project file browsing and expose it only as AppFS state.

## Acceptance Criteria

1. User can select a project folder in WebUI.
2. Dashboard can start AppFS for that project.
3. Compose file lives at `projectRoot/.appfs-compose.yaml`.
4. Mountpoint is `projectRoot/.appfs`.
5. Project files remain readable and editable in the real project folder.
6. Dashboard agent list is grouped by project.
7. `.appfs` is ignored by normal project file detection.
8. The design remains compatible with a later Electron shell.

