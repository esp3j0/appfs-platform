# APPFS Electron 壳层实施计划

> **For coder:** 按顺序实现。Electron 只做壳层，不把 AppFS runtime 逻辑搬出 `dashboard/server`。

**Goal:** 把现有 dashboard 包成桌面壳，补上 project picker、recent projects、compose 编辑保存、packaged binary 路径解析，以及清晰的启动 / 退出语义，同时保持 `dashboard/server` 作为 project runtime 的 source of truth。

**Architecture:** Electron main 负责窗口、原生对话框、shell store、renderer/API origin 选择和 packaged path 解析；`dashboard/server` 继续负责 project registry、compose 校验、AppFS 启停和 agent 启动；renderer 负责 UI、selected project 状态，以及调用后端 API / preload API。

**Core path model:**

1. `projectRoot` = 用户真实工作区
2. `projectRoot/.appfs-compose.yaml` = compose source of truth
3. `projectRoot/.appfs` = AppFS mountpoint only
4. `Electron userData` = recent projects / window state / launch profile
5. `selectedProjectId` = renderer runtime state, derived from `POST /api/projects/open`

**Non-goal:** 这一步不要做 overlay，不要把 runtime state 写回 project 目录。

---

## Cross-Cutting Decisions Before Coding

1. `dashboard/server` 必须能在没有已选项目的情况下启动，保持空 project registry。
2. packaged 模式建议由 `dashboard/server` serve `dashboard/dist`，Electron 加载 `http://127.0.0.1:<port>`，renderer 使用同源 `/api` 和 `/api/events`。
3. dev 模式可以继续用 Vite proxy，但 API origin 仍应能被 shell 明确注入或配置。
4. packaged 模式需要两个二进制：AppFS CLI 和 managed agent。
5. window close 和 app quit 要分开，app quit 时要确认并清理 server-owned runtime 子进程。
6. compose 校验必须是 AppFS schema 校验，不是只做 YAML 语法校验。

---

## Task 1: Add Electron Shell Package

**Files:**

- Create: `desktop/package.json`
- Create: `desktop/src/main.ts`
- Create: `desktop/src/preload.ts`
- Create: `desktop/src/shell-store.ts`
- Create: `desktop/src/server-launcher.ts`
- Modify: `dashboard/server/src/index.ts`
- Modify: `dashboard/server/package.json`
- Modify: `dashboard/package.json`
- Optional: add `dashboard/server/src/static-dashboard.ts` or equivalent

**Step 1: Add bootstrap tests**

Write tests for:

1. single instance lock is acquired;
2. shell store can persist recent projects and window bounds;
3. launcher can resolve dev vs packaged runtime mode;
4. server boot waits for readiness before showing the window;
5. `dashboard/server` can start with an empty project registry when no project root is supplied;
6. packaged launch URL and API origin resolve to the same local HTTP origin;
7. app quit cleanup sends a shutdown signal that lets server-owned child processes exit.

**Step 2: Implement minimal shell bootstrap**

Implement:

1. Electron main window creation;
2. preload bridge using strict Context Isolation and safe IPC-only channels;
3. shell state load / save;
4. dashboard server launch / stop;
5. empty-registry server boot;
6. packaged static dashboard serving from `dashboard/server`;
7. dev load URL vs packaged load URL resolution;
8. app quit cleanup.

Server launcher rules:

1. choose an available localhost port instead of assuming `3100` is free;
2. pass the chosen `PORT`, `HOST`, launch profile, and binary env into the server;
3. wait for `GET /api/projects` before showing the window;
4. in packaged mode, load the renderer from the server HTTP origin, not from a bare `file://` URL.

Server rules:

1. missing `DUMP_DIR` should no longer be fatal in desktop mode;
2. initial registry can be empty;
3. if a project root is supplied for legacy web/debug use, preserve the old auto-register behavior;
4. expose a graceful shutdown path that stops managed agents and project runtimes.

**Step 3: Verify**

Run the desktop package test/build command chosen by the repo owner.

Expected:

- shell starts without manual path wiring;
- shell store round-trips correctly;
- server launch is isolated from renderer;
- port conflicts are auto-resolved;
- no zombie processes remain after closing the desktop app;
- first run without recent projects reaches the renderer instead of exiting in server bootstrap.

---

## Task 2: Add Project Picker and Recent Projects

**Files:**

- Modify: `desktop/src/preload.ts`
- Modify: `desktop/src/shell-store.ts`
- Modify: `dashboard/src/App.tsx`
- Modify: `dashboard/src/types.ts`
- Modify: `dashboard/src/components/TopBar.tsx`
- Modify: `dashboard/src/components/AgentSidebar.tsx`
- Add: `dashboard/src/components/ProjectPicker.tsx` or equivalent

**Step 1: Add failing tests**

Cover:

1. selecting a folder registers a project root;
2. selected project is written into recent projects only after server registration succeeds;
3. recent projects survive app restart;
4. folder picker rejects files and empty selections;
5. selected project state is restored from `lastSelectedProjectRoot`;
6. deleted / inaccessible recent project paths produce a visible recovery state.

**Step 2: Implement minimal behavior**

Add a narrow preload API for:

- choose project folder
- get recent project list
- remove recent project
- persist selected project root
- get shell/runtime profile metadata needed by the renderer

The renderer should:

1. show recent projects before any runtime is running;
2. call `POST /api/projects/open` for a chosen or recent project;
3. activate the returned `projectId` as the selected project in the dashboard;
4. keep the list ordered by last opened time;
5. not store runtime state in the shell store;
6. pass selected project context to compose editing, start/stop controls, and spawn agent flows;
7. dedupe recent projects by normalized platform path.

Do not put project registration logic in Electron main except as a thin HTTP call to `dashboard/server` if the UI flow requires it. `dashboard/server` remains the source of truth for project records.

**Step 3: Verify**

Run the desktop unit tests and one manual smoke:

1. open a project
2. quit shell
3. reopen shell
4. confirm recent project still appears
5. confirm selected project is re-opened through the server and receives a fresh `projectId`

---

## Task 3: Add Compose Read / Write Surface

**Files:**

- Modify: `dashboard/server/src/routes/projects.ts`
- Add: `dashboard/server/src/routes/project-compose.ts` or equivalent
- Add: `dashboard/src/components/ProjectComposeEditor.tsx`
- Modify: `dashboard/src/components/TopBar.tsx`
- Modify: `dashboard/src/App.tsx`
- Modify: `appfs/cli/src/opts.rs` if no compose validate command exists
- Modify: `appfs/cli/src/main.rs` if no compose validate command exists
- Modify: `appfs/cli/src/cmd/appfs.rs` if no compose validate command exists

**Step 1: Add failing route tests**

Cover:

1. read current compose text for a project;
2. save compose text atomically;
3. reject invalid YAML before write or before start;
4. reject YAML that is syntactically valid but invalid against AppFS compose schema;
5. preserve dirty draft if save fails;
6. return a clear error when compose is missing;
7. do not overwrite the old compose file when validation fails;
8. validate unknown fields using the same rules as the AppFS compose parser.

**Step 2: Implement the compose pipeline**

The editor should:

1. load compose from `projectRoot/.appfs-compose.yaml`;
2. keep a dirty draft per project;
3. validate before save;
4. write atomically;
5. refresh project state after save.

Server route shape can be:

1. `GET /api/projects/:projectId/compose`
2. `POST /api/projects/:projectId/compose/validate`
3. `PUT /api/projects/:projectId/compose`

Validation rules:

1. use AppFS compose schema validation, not only generic YAML parsing;
2. if AppFS CLI does not already expose validation, add `appfs compose validate -f <file>`;
3. write candidate content to a temp file in the project directory, validate that temp file, then rename over `.appfs-compose.yaml`;
4. keep temp files out of `.appfs`;
5. keep parse and mountpoint checks in `dashboard/server`, not in Electron main.

**Step 3: Verify**

Run:

- server route tests for compose read/write
- AppFS compose validate tests if a new CLI command is added
- dashboard build

Expected:

- compose can be edited and saved from the same window;
- invalid YAML does not silently overwrite the source file;
- schema-invalid compose does not silently overwrite the source file.

---

## Task 4: Wire Packaged AppFS CLI, Agent Binary, and Launch Profiles

**Files:**

- Modify: `desktop/src/server-launcher.ts`
- Modify: `dashboard/server/src/index.ts`
- Modify: `dashboard/server/src/process-manager.ts`
- Modify: `dashboard/src/types.ts`
- Modify: `dashboard/src/components/AgentSidebar.tsx`

**Step 1: Add failing tests**

Cover:

1. dev mode still uses cargo launch specs for AppFS CLI and managed agent unless env overrides are set;
2. packaged mode resolves a bundled AppFS CLI binary path;
3. packaged mode resolves a bundled managed agent binary path;
4. project start uses the packaged AppFS CLI path instead of cargo;
5. default spawn config uses the packaged managed agent binary path instead of cargo;
6. no absolute binary path is persisted into project files, compose files, or recent projects;
7. spawn config keeps the same shape for renderer calls, with optional `projectId` support.

**Step 2: Implement runtime profile resolution**

Add a shell runtime profile with two modes:

1. `dev` -> cargo / manifest path by default
2. `packaged` -> resolved AppFS CLI binary path + resolved managed agent binary path

The shell should pass the resolved launch profile into the server as env / bootstrap config:

1. `APPFS_CLI_BIN` or equivalent server bootstrap field for `appfs compose up`;
2. `DASHBOARD_AGENT_BIN` or equivalent server bootstrap field for default managed agent spawns;
3. existing `DASHBOARD_AGENT_MANIFEST` and cargo defaults remain valid in dev mode.

The renderer should still work with the existing `SpawnConfig` shape plus optional project fields:

1. `projectId?: string`
2. `projectRoot?: string`

Do not expose bundled absolute binary paths as editable UI fields in the normal packaged flow.

**Step 3: Verify**

Run the spawn tests and one packaged-mode smoke check.

Expected:

- packaged mode no longer depends on hand-set local paths;
- packaged mode can start AppFS runtime and managed agents without Rust/Cargo;
- dev workflow is unchanged.

---

## Task 5: Shell / UI Integration and Smoke Tests

**Files:**

- Modify: `dashboard/src/App.tsx`
- Modify: `dashboard/src/components/TopBar.tsx`
- Modify: `dashboard/src/components/AgentSidebar.tsx`
- Modify: `dashboard/src/components/ProjectComposeEditor.tsx`
- Add: `desktop/e2e/*` or `integration/*` smoke coverage

**Step 1: Add end-to-end coverage**

Cover:

1. open project
2. edit compose
3. save compose
4. start AppFS
5. spawn agent
6. stop AppFS
7. reopen recent project
8. close window while runtime is running and confirm behavior
9. quit app and verify no server-owned runtime child is left behind

**Step 2: Implement the shell-first UX**

The first useful screen should be the real project UI, not a marketing page.
If no project is selected yet, show the project picker / recent projects state.

Selected project UX rules:

1. top bar should show the selected project display name and runtime status;
2. sidebar project groups can remain visible, but start/stop/spawn should clearly target the selected project;
3. compose editor should switch drafts when selected project changes;
4. if no project is selected, disable runtime and spawn actions that require a project.

**Step 3: Verify**

Run:

1. desktop smoke test
2. dashboard build
3. server tests for project runtime and compose routes
4. packaged-mode smoke for both AppFS CLI and managed agent launch profile

Expected:

- the shell feels like one app;
- project selection, compose editing, runtime control and recent projects all work together;
- `dashboard/server` still remains the authoritative control plane;
- closing/quitting behavior is explicit and leaves no orphaned server-owned runtime process.

## Acceptance Criteria

1. Electron can open a project folder with a native dialog.
2. Recent projects survive restart.
3. `.appfs-compose.yaml` can be edited and saved in-app.
4. AppFS can be started and stopped from the shell.
5. Packaged mode resolves binary paths without manual setup.
6. Dashboard/server still owns project runtime state.
7. The design stays compatible with the existing web dashboard workflow.
8. Server can boot with an empty project registry.
9. Packaged renderer can reach HTTP and SSE APIs without relying on Vite proxy.
10. Packaged mode resolves both AppFS CLI and managed agent binaries.
11. Compose save performs AppFS schema validation and atomic replacement.
12. Runtime cleanup on app quit is explicit and verified.

