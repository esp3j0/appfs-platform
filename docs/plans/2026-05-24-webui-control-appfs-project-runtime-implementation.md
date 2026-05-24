# WebUI-Controlled AppFS Project Runtime Implementation Plan

> **For agentic workers:** implement this plan in order. Do not jump to Electron yet.

**Goal:** Let the current dashboard choose a project folder, start and stop `appfs compose up` for that project, and group agents by project before we wrap the whole stack into Electron.

**Core path model:**

1. project root = real user workspace
2. `projectRoot/.appfs-compose.yaml` = compose config
3. `projectRoot/.appfs` = AppFS mountpoint only
4. `projectRoot/.claw` = existing session storage, unchanged

**Non-goal:** overlay the entire project root in this phase.

---

## Task 1: Add Project Runtime Registry

**Files:**

- Create: `dashboard/server/src/project-registry.ts`
- Modify: `dashboard/server/src/index.ts`
- Modify: `dashboard/server/src/types.ts`
- Modify: `dashboard/server/src/agent-registry.ts`

**Step 1: Write the failing test**

Add server tests that assert:

1. a project can be registered from a project root path;
2. the registry derives `composeFilePath` as `projectRoot/.appfs-compose.yaml`;
3. the registry derives `mountRoot` as `projectRoot/.appfs`;
4. project grouping uses a stable project key, not a session fingerprint;
5. `.appfs` and `.claw` are excluded from normal project content scanning.

**Step 2: Run the test and verify it fails**

Run:

```powershell
npx tsx --test dashboard/server/src/project-registry.test.ts
```

Expected:

- no project registry exists yet, so the tests fail.

**Step 3: Write minimal implementation**

Introduce a `ProjectRegistry` that tracks:

```ts
interface ProjectRecord {
  projectId: string;
  projectRoot: string;
  composeFilePath: string;
  mountRoot: string;
  status: 'stopped' | 'starting' | 'running' | 'error';
}
```

The registry should:

1. normalize project roots;
2. derive the hidden mountpoint and compose file paths;
3. store a list of agent session IDs per project;
4. provide lookup by absolute project root.

**Step 4: Run the test to verify it passes**

Run:

```powershell
npx tsx --test dashboard/server/src/project-registry.test.ts
```

Expected:

- project records resolve correctly;
- `.appfs` stays outside normal source scanning.

**Step 5: Commit**

Commit message:

```bash
git commit -m "feat(dashboard): add project runtime registry"
```

---

## Task 2: Add AppFS Project Start/Stop APIs

**Files:**

- Create: `dashboard/server/src/routes/projects.ts`
- Modify: `dashboard/server/src/index.ts`
- Modify: `dashboard/server/src/process-manager.ts`

**Step 1: Write the failing test**

Add route tests for:

1. `GET /api/projects`
2. `POST /api/projects/open`
3. `POST /api/projects/:projectId/start`
4. `POST /api/projects/:projectId/stop`
5. `GET /api/projects/:projectId/status`

The tests should assert:

- the server accepts a selected project root;
- it resolves `projectRoot/.appfs-compose.yaml`;
- it refuses to start if `projectRoot/.appfs` is already a non-empty directory;
- it returns a clear error if the compose file is missing.

**Step 2: Run the test and verify it fails**

Run:

```powershell
npx tsx --test dashboard/server/src/routes/projects.test.ts
```

Expected:

- no project routes exist yet, so the tests fail.

**Step 3: Write minimal implementation**

Implement:

1. project open/select flow;
2. compose start flow;
3. compose stop flow;
4. runtime status reporting.

The start flow should:

1. validate the project root;
2. resolve `projectRoot/.appfs-compose.yaml`;
3. validate `projectRoot/.appfs` is absent or empty;
4. launch `appfs compose up` for that project.

The stop flow should:

1. stop managed agents in that project;
2. stop the AppFS runtime;
3. unmount the project mountpoint;
4. leave the project files intact.

**Step 4: Run the test to verify it passes**

Run:

```powershell
npx tsx --test dashboard/server/src/routes/projects.test.ts
```

Expected:

- projects can be started and stopped deterministically;
- mountpoint hygiene errors are explicit.

**Step 5: Commit**

Commit message:

```bash
git commit -m "feat(dashboard): add project appfs lifecycle routes"
```

---

## Task 3: Make Agent Spawn Project-Scoped

**Files:**

- Modify: `dashboard/server/src/process-manager.ts`
- Modify: `dashboard/server/src/agent-registry.ts`
- Modify: `dashboard/src/components/AgentSidebar.tsx`
- Modify: `dashboard/src/components/PlaygroundPanel.tsx`

**Step 1: Write the failing test**

Add tests that verify:

1. a managed agent can be spawned for a selected project;
2. `cwd` is the project root;
3. `appfsMountRoot` is `projectRoot/.appfs`;
4. the agent registry stores `projectId` or `projectRoot`;
5. the sidebar groups agents by project.

**Step 2: Run the test and verify it fails**

Run:

```powershell
npm --prefix dashboard/server test
```

Expected:

- the project-scoped agent assertions fail until the code is wired.

**Step 3: Write minimal implementation**

Update the spawn config so dashboard-managed agents receive:

1. `cwd = projectRoot`
2. `appfsMountRoot = projectRoot/.appfs`
3. `APPFS_PRINCIPAL_ID`
4. the project identifier in registry metadata

Update the sidebar so it renders:

1. project header
2. runtime status
3. agents underneath the project
4. principal/session/control mode per agent

**Step 4: Run the test to verify it passes**

Run:

```powershell
npm --prefix dashboard/server test
npm --prefix dashboard run build
```

Expected:

- agents are grouped by project;
- spawn/resume actions stay scoped to the selected project.

**Step 5: Commit**

Commit message:

```bash
git commit -m "feat(dashboard): scope agents by project runtime"
```

---

## Task 4: Hide `.appfs` From Normal Project Browsing

**Files:**

- Modify: `dashboard/server/src/file-watcher.ts`
- Modify: `dashboard/server/src/routes/mounted-apps.ts`
- Modify: `dashboard/src/components/InfoPanel.tsx`
- Modify: `dashboard/src/components/AgentSidebar.tsx`

**Step 1: Write the failing test**

Add tests that assert:

1. project file scanning ignores `.appfs`;
2. project file scanning ignores `.claw`;
3. AppFS-specific browsing can still target `projectRoot/.appfs`;
4. a path under `.appfs` is not shown as a normal project source file.

**Step 2: Run the test and verify it fails**

Run:

```powershell
npm --prefix dashboard/server test
```

Expected:

- the filtering logic is not yet in place.

**Step 3: Write minimal implementation**

Implement two distinct path walkers:

1. source-file walker for the project UI
2. AppFS runtime walker for control/state files

The source-file walker must exclude:

1. `.appfs`
2. `.claw`
3. other hidden runtime-only folders if they are introduced later

The AppFS walker should use `projectRoot/.appfs` as the root when the UI needs AppFS internals.

**Step 4: Run the test to verify it passes**

Run:

```powershell
npm --prefix dashboard/server test
```

Expected:

- normal project browsing is clean;
- AppFS content is still reachable through the control panel.

**Step 5: Commit**

Commit message:

```bash
git commit -m "feat(dashboard): hide appfs runtime folders from project browsing"
```

---

## Task 5: Manual Verification

1. pick an existing project folder in the WebUI;
2. verify the dashboard uses `projectRoot/.appfs-compose.yaml`;
3. verify `projectRoot/.appfs` is the mountpoint only;
4. start AppFS from the UI;
5. spawn two agents under the same project;
6. confirm the sidebar groups them under one project header;
7. confirm project source browsing does not show `.appfs` or `.claw`;
8. confirm AppFS files are still reachable through the AppFS view.

## Acceptance Criteria

1. dashboard can open a project folder and start AppFS;
2. compose config is stored in `projectRoot/.appfs-compose.yaml`;
3. mountpoint is `projectRoot/.appfs`;
4. source browsing does not treat `.appfs` as project content;
5. agents are grouped by project, not only by session;
6. managed agent lifecycle remains intact;
7. the design stays Electron-ready.

