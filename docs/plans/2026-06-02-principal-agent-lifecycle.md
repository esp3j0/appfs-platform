# Principal Agent Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build one project-scoped lifecycle surface for creating, deleting, starting, stopping, resuming, and observing AppFS principal agents.

**Architecture:** AppFS remains the source of truth for principal control-plane records and status view files. The dashboard server adds a `PrincipalLifecycleService` that appends AppFS principal action lines, reads runtime-maintained principal views, and orchestrates `AgentProcessManager` for headless agent processes. Dashboard UI and future Claude Code-style `Agent` teammate tools should call this service instead of inventing separate lifecycle paths.

**Tech Stack:** TypeScript, Fastify, Node test runner, AppFS `.act` JSONL control files, dashboard `ProjectRegistry`, `AgentRegistry`, `AgentProcessManager`, React.

---

## Current State

Relevant existing files:

- `appfs/cli/src/cmd/appfs/supervisor_control.rs`
- `appfs/cli/src/cmd/appfs/runtime_supervisor.rs`
- `appfs/cli/src/cmd/appfs/registry.rs`
- `appfs-agent/rust/crates/runtime/src/appfs.rs`
- `dashboard/server/src/routes/principals.ts`
- `dashboard/server/src/routes/process.ts`
- `dashboard/server/src/process-manager.ts`
- `dashboard/server/src/project-registry.ts`
- `dashboard/server/src/index.ts`
- `dashboard/src/components/AgentSidebar.tsx`
- `dashboard/src/components/AgentItem.tsx`

Existing AppFS control actions:

- `_appfs/principals/create_principal.act`
- `_appfs/principals/update_principal.act`
- `_appfs/principals/delete_principal.act`
- `_appfs/principals/attach_principal.act`
- `_appfs/principals/detach_principal.act`

Existing AppFS status views:

- `_appfs/principals.registry.json`
- `_appfs/principals/<principal-id>.res.json`
- `_appfs/principals/status.res.json`

Primary gap:

- The dashboard server has session/process-centric routes, but no principal-centric lifecycle API.
- `GET /api/principals` is not project-scoped.
- Start/resume/stop operations are keyed by session id, not principal id.
- Future teammate-agent tooling has no single lifecycle service to reuse.

## Design Decisions

1. AppFS control-plane files are authoritative. The dashboard must never edit `principals.registry.json` directly.
2. Lifecycle APIs are project-scoped: use `/api/projects/:projectId/principals/...`.
3. Only one live dashboard-managed agent should run per principal.
4. If a principal already has a managed running agent, `start` and `resume` return that existing agent instead of spawning another.
5. `delete` rejects online principals unless a future explicit `force` option is added.
6. `default` principal deletion is rejected.
7. `resume` uses a caller-provided `sessionId` when present; otherwise it selects the latest discovered session for the principal in the same project.
8. True attach takeover is deferred. Initial lifecycle uses explicit stop-then-start semantics.
9. The legacy `/api/principals` route can remain temporarily as a compatibility wrapper, but project-scoped routes are the canonical surface.

## Public API Contract

### List principals

`GET /api/projects/:projectId/principals`

Response:

```json
{
  "version": 1,
  "default_principal_id": "default",
  "principals": [
    {
      "principal_id": "default",
      "display_name": "default",
      "description": null,
      "kind": "agent",
      "created_at": "2026-06-02T00:00:00Z",
      "updated_at": "2026-06-02T00:00:00Z",
      "online": true,
      "status": "busy",
      "agent_status": {
        "state": "running",
        "current_task_preview": "Inspect code",
        "session_id": "session-123"
      },
      "pid": 1234,
      "sessionId": "session-123",
      "model": "claude-opus-4-6",
      "permissionMode": "dangerous"
    }
  ]
}
```

### Create principal

`POST /api/projects/:projectId/principals`

Request:

```json
{
  "principalId": "coder",
  "displayName": "coder",
  "description": "Coding teammate",
  "kind": "agent"
}
```

Response:

```json
{
  "status": "created",
  "principal": {
    "principal_id": "coder"
  }
}
```

### Delete principal

`DELETE /api/projects/:projectId/principals/:principalId`

Response:

```json
{
  "status": "deleted",
  "principalId": "coder"
}
```

### Start principal agent

`POST /api/projects/:projectId/principals/:principalId/start`

Request:

```json
{
  "modelProviderId": "anthropic-default",
  "modelId": "claude-opus-4-6",
  "model": "claude-opus-4-6",
  "contextWindowTokens": 200000,
  "maxOutputTokens": 8192,
  "permissionMode": "dangerous"
}
```

Response when newly spawned:

```json
{
  "status": "spawning",
  "spawnId": "spawn-...",
  "principalId": "coder"
}
```

Response when already running:

```json
{
  "status": "already-running",
  "principalId": "coder",
  "sessionId": "session-123"
}
```

### Stop principal agent

`POST /api/projects/:projectId/principals/:principalId/stop`

Response:

```json
{
  "status": "stopping",
  "principalId": "coder",
  "sessionId": "session-123"
}
```

### Resume principal agent

`POST /api/projects/:projectId/principals/:principalId/resume`

Request:

```json
{
  "sessionId": "session-123"
}
```

Response:

```json
{
  "status": "spawning",
  "spawnId": "spawn-...",
  "principalId": "coder",
  "sessionId": "session-123"
}
```

## Task 1: Add Principal Lifecycle Types

**Files:**

- Create: `dashboard/server/src/principal-lifecycle.ts`
- Test: `dashboard/server/src/principal-lifecycle.test.ts`

**Step 1: Write the failing tests**

Add tests for safe principal id validation and project mount path resolution.

```ts
import { describe, it } from 'node:test';
import assert from 'node:assert';
import { normalizePrincipalId, principalControlDir } from './principal-lifecycle.js';

describe('PrincipalLifecycle helpers', () => {
  it('normalizes safe principal ids', () => {
    assert.strictEqual(normalizePrincipalId(' coder '), 'coder');
    assert.strictEqual(normalizePrincipalId('team.alpha-1'), 'team.alpha-1');
    assert.throws(() => normalizePrincipalId('..'), /Invalid principalId/);
    assert.throws(() => normalizePrincipalId('bad/name'), /Invalid principalId/);
  });

  it('resolves the project principal control directory', () => {
    assert.match(
      principalControlDir('C:/repo/project/.appfs').replace(/\\/g, '/'),
      /\/\.appfs\/_appfs\/principals$/
    );
  });
});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: FAIL because `principal-lifecycle.ts` does not exist.

**Step 3: Write minimal implementation**

Create `dashboard/server/src/principal-lifecycle.ts`:

```ts
import path from 'node:path';

const SAFE_PRINCIPAL_ID = /^[A-Za-z0-9_.-]{1,128}$/;

export function normalizePrincipalId(input: string): string {
  const principalId = input.trim();
  if (!SAFE_PRINCIPAL_ID.test(principalId) || principalId === '.' || principalId === '..') {
    throw new PrincipalLifecycleError(400, `Invalid principalId: ${input}`);
  }
  return principalId;
}

export function principalControlDir(mountRoot: string): string {
  return path.join(mountRoot, '_appfs', 'principals');
}

export class PrincipalLifecycleError extends Error {
  constructor(public statusCode: number, message: string) {
    super(message);
    this.name = 'PrincipalLifecycleError';
  }
}
```

**Step 4: Run test to verify it passes**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/principal-lifecycle.ts dashboard/server/src/principal-lifecycle.test.ts
git commit -m "feat: add principal lifecycle helpers"
```

## Task 2: Read Project-Scoped Principal Views

**Files:**

- Modify: `dashboard/server/src/principal-lifecycle.ts`
- Test: `dashboard/server/src/principal-lifecycle.test.ts`

**Step 1: Write the failing test**

Add a test that writes fake principal registry and status files under a temporary project mount and verifies merged results.

```ts
it('reads project-scoped principal registry and aggregate status', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'principal-lifecycle-'));
  const mountRoot = path.join(temp, '.appfs');
  const principalsDir = path.join(mountRoot, '_appfs', 'principals');
  fs.mkdirSync(principalsDir, { recursive: true });
  fs.writeFileSync(path.join(mountRoot, '_appfs', 'principals.registry.json'), JSON.stringify({
    version: 1,
    default_principal_id: 'default',
    principals: [{ principal_id: 'default', display_name: 'default', kind: 'agent' }]
  }));
  fs.writeFileSync(path.join(principalsDir, 'status.res.json'), JSON.stringify({
    version: 1,
    principals: [{ principal_id: 'default', state: 'idle', session_id: 'session-a' }]
  }));

  const views = readPrincipalViews(mountRoot);
  assert.strictEqual(views.default_principal_id, 'default');
  assert.strictEqual(views.principals[0].principal_id, 'default');
  assert.strictEqual(views.principals[0].agent_status?.state, 'idle');

  fs.rmSync(temp, { recursive: true, force: true });
});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: FAIL because `readPrincipalViews` is not implemented.

**Step 3: Write minimal implementation**

Add interfaces and `readPrincipalViews(mountRoot)`:

```ts
import fs from 'node:fs';

export interface PrincipalRegistryPrincipal {
  principal_id: string;
  display_name?: string;
  description?: string | null;
  kind?: string;
  created_at?: string;
  updated_at?: string;
  agent_status?: Record<string, unknown> | null;
}

export interface PrincipalRegistryDoc {
  version: number;
  default_principal_id?: string;
  principals: PrincipalRegistryPrincipal[];
}

export function readPrincipalViews(mountRoot: string): PrincipalRegistryDoc {
  const registryPath = path.join(mountRoot, '_appfs', 'principals.registry.json');
  const statusPath = path.join(principalControlDir(mountRoot), 'status.res.json');
  const doc: PrincipalRegistryDoc = fs.existsSync(registryPath)
    ? JSON.parse(fs.readFileSync(registryPath, 'utf8'))
    : { version: 1, principals: [] };
  const statusDoc = fs.existsSync(statusPath)
    ? JSON.parse(fs.readFileSync(statusPath, 'utf8'))
    : null;
  const statusByPrincipal = new Map<string, Record<string, unknown>>();
  for (const entry of statusDoc?.principals ?? []) {
    if (entry?.principal_id) statusByPrincipal.set(entry.principal_id, entry);
  }
  return {
    ...doc,
    principals: doc.principals.map(principal => ({
      ...principal,
      agent_status: principal.agent_status ?? statusByPrincipal.get(principal.principal_id) ?? null,
    })),
  };
}
```

**Step 4: Run test to verify it passes**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/principal-lifecycle.ts dashboard/server/src/principal-lifecycle.test.ts
git commit -m "feat: read project scoped principal views"
```

## Task 3: Append AppFS Principal Action Lines

**Files:**

- Modify: `dashboard/server/src/principal-lifecycle.ts`
- Test: `dashboard/server/src/principal-lifecycle.test.ts`

**Step 1: Write the failing test**

```ts
it('appends create principal action JSONL', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'principal-action-'));
  const mountRoot = path.join(temp, '.appfs');

  appendPrincipalAction(mountRoot, 'create_principal.act', {
    principal_id: 'coder',
    display_name: 'coder',
    kind: 'agent',
  });

  const actionPath = path.join(mountRoot, '_appfs', 'principals', 'create_principal.act');
  const lines = fs.readFileSync(actionPath, 'utf8').trim().split(/\r?\n/);
  const parsed = JSON.parse(lines[0]);
  assert.strictEqual(parsed.principal_id, 'coder');
  assert.ok(parsed.client_token);

  fs.rmSync(temp, { recursive: true, force: true });
});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: FAIL because `appendPrincipalAction` is not implemented.

**Step 3: Write minimal implementation**

Add:

```ts
import crypto from 'node:crypto';

export function appendPrincipalAction(
  mountRoot: string,
  actionFile: 'create_principal.act' | 'update_principal.act' | 'delete_principal.act',
  payload: Record<string, unknown>,
): string {
  const dir = principalControlDir(mountRoot);
  fs.mkdirSync(dir, { recursive: true });
  const clientToken = typeof payload.client_token === 'string'
    ? payload.client_token
    : `dashboard-${Date.now()}-${crypto.randomUUID()}`;
  const line = JSON.stringify({ ...payload, client_token: clientToken }) + '\n';
  fs.appendFileSync(path.join(dir, actionFile), line, 'utf8');
  return clientToken;
}
```

**Step 4: Run test to verify it passes**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/principal-lifecycle.ts dashboard/server/src/principal-lifecycle.test.ts
git commit -m "feat: append principal lifecycle actions"
```

## Task 4: Add ProcessManager Principal Lookup Helpers

**Files:**

- Modify: `dashboard/server/src/process-manager.ts`
- Test: `dashboard/server/src/process-manager.test.ts`

**Step 1: Write the failing test**

Add a focused test for exported helper behavior if direct process spawning is hard to instantiate. Prefer adding methods and testing them with a small fake managed map only if visibility allows. Otherwise add a test for a pure principal matching helper:

```ts
import { samePrincipalScope } from './process-manager.js';

it('matches principal within a project scope', () => {
  assert.strictEqual(samePrincipalScope(
    { principalId: 'coder', projectId: 'project-a' },
    'coder',
    'project-a',
  ), true);
  assert.strictEqual(samePrincipalScope(
    { principalId: 'coder', projectId: 'project-a' },
    'coder',
    'project-b',
  ), false);
});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\process-manager.test.ts
Pop-Location
```

Expected: FAIL because `samePrincipalScope` does not exist.

**Step 3: Write minimal implementation**

Add:

```ts
export function samePrincipalScope(
  candidate: { principalId: string; projectId?: string },
  principalId: string,
  projectId?: string,
): boolean {
  return candidate.principalId === principalId
    && (!projectId || candidate.projectId === projectId);
}
```

Then add class methods:

```ts
findManagedAgentByPrincipal(principalId: string, projectId?: string) {
  for (const agent of this.agents.values()) {
    if (samePrincipalScope(agent.spawnConfig, principalId, projectId)) {
      return {
        pid: agent.process.pid,
        sessionId: agent.sessionId,
        status: agent.status,
        principalId: agent.spawnConfig.principalId,
        model: agent.spawnConfig.model,
        permissionMode: agent.spawnConfig.permissionMode,
      };
    }
  }
  return null;
}

stopPrincipal(principalId: string, projectId?: string): { sessionId: string | null } | null {
  for (const agent of this.agents.values()) {
    if (samePrincipalScope(agent.spawnConfig, principalId, projectId)) {
      const sessionId = agent.sessionId;
      void terminateChildProcessTree(agent.process, {
        label: `agent ${agent.sessionId ?? principalId}`,
        gracefulTimeoutMs: 5000,
      });
      return { sessionId };
    }
  }
  return null;
}
```

**Step 4: Run test to verify it passes**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\process-manager.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/process-manager.ts dashboard/server/src/process-manager.test.ts
git commit -m "feat: lookup managed agents by principal"
```

## Task 5: Implement PrincipalLifecycleService List/Create/Delete

**Files:**

- Modify: `dashboard/server/src/principal-lifecycle.ts`
- Test: `dashboard/server/src/principal-lifecycle.test.ts`

**Step 1: Write the failing tests**

Add tests:

```ts
it('creates a principal by appending an AppFS create action', async () => {
  const service = new PrincipalLifecycleService({ projectRegistry, agentRegistry, processManager });
  const result = await service.createPrincipal(project.projectId, {
    principalId: 'coder',
    displayName: 'coder',
    description: 'Coding agent',
    kind: 'agent',
  });
  assert.strictEqual(result.status, 'created');
  const action = fs.readFileSync(path.join(project.mountRoot, '_appfs', 'principals', 'create_principal.act'), 'utf8');
  assert.match(action, /"principal_id":"coder"/);
});

it('rejects deleting default principal', async () => {
  const service = new PrincipalLifecycleService({ projectRegistry, agentRegistry, processManager });
  await assert.rejects(
    () => service.deletePrincipal(project.projectId, 'default'),
    /Cannot delete default principal/
  );
});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: FAIL because `PrincipalLifecycleService` is not implemented.

**Step 3: Write minimal implementation**

Add constructor dependencies:

```ts
import type { ProjectRegistry } from './project-registry.js';
import type { AgentRegistry } from './agent-registry.js';
import type { AgentProcessManager } from './process-manager.js';

export class PrincipalLifecycleService {
  constructor(private deps: {
    projectRegistry: ProjectRegistry;
    agentRegistry: AgentRegistry;
    processManager: AgentProcessManager;
  }) {}

  listPrincipals(projectId: string) {
    const project = this.requireProject(projectId);
    const doc = readPrincipalViews(project.mountRoot);
    const managedAgents = this.deps.processManager.getManagedAgents();
    const registryAgents = this.deps.agentRegistry.getAgents()
      .filter(agent => agent.projectId === projectId);

    return {
      ...doc,
      principals: doc.principals.map(principal => {
        const managed = managedAgents.find(agent => agent.principalId === principal.principal_id);
        const discovered = registryAgents
          .filter(agent => agent.principalId === principal.principal_id)
          .sort((a, b) => b.startedAt - a.startedAt)[0];
        const active = managed ?? discovered;
        return {
          ...principal,
          online: active ? active.status !== 'offline' : false,
          status: active?.status ?? 'offline',
          pid: active?.pid && active.pid !== 0 ? active.pid : undefined,
          sessionId: active?.sessionId ?? null,
          model: active?.model,
          permissionMode: active && 'permissionMode' in active ? active.permissionMode : undefined,
        };
      }),
    };
  }

  async createPrincipal(projectId: string, input: {
    principalId: string;
    displayName?: string;
    description?: string;
    kind?: string;
  }) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(input.principalId);
    appendPrincipalAction(project.mountRoot, 'create_principal.act', {
      principal_id: principalId,
      display_name: input.displayName ?? principalId,
      description: input.description,
      kind: input.kind ?? 'agent',
    });
    return { status: 'created' as const, principal: { principal_id: principalId } };
  }

  async deletePrincipal(projectId: string, principalIdInput: string) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(principalIdInput);
    if (principalId === 'default') {
      throw new PrincipalLifecycleError(400, 'Cannot delete default principal');
    }
    const managed = this.deps.processManager.findManagedAgentByPrincipal(principalId, projectId);
    if (managed && managed.status !== 'starting') {
      throw new PrincipalLifecycleError(409, `Principal ${principalId} has a running managed agent`);
    }
    appendPrincipalAction(project.mountRoot, 'delete_principal.act', { principal_id: principalId });
    return { status: 'deleted' as const, principalId };
  }

  private requireProject(projectId: string) {
    const project = this.deps.projectRegistry.getProject(projectId);
    if (!project) {
      throw new PrincipalLifecycleError(404, `Project ${projectId} not found`);
    }
    return project;
  }
}
```

**Step 4: Run test to verify it passes**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/principal-lifecycle.ts dashboard/server/src/principal-lifecycle.test.ts
git commit -m "feat: create and delete principals from dashboard"
```

## Task 6: Implement Start/Stop/Resume in PrincipalLifecycleService

**Files:**

- Modify: `dashboard/server/src/principal-lifecycle.ts`
- Test: `dashboard/server/src/principal-lifecycle.test.ts`

**Step 1: Write the failing tests**

Use fakes for `AgentRegistry` and `AgentProcessManager`.

Required tests:

```ts
it('startPrincipal returns existing managed agent instead of spawning duplicate', async () => {});
it('startPrincipal spawns new agent with project-scoped config', async () => {});
it('stopPrincipal stops managed agent by principal id', async () => {});
it('resumePrincipal chooses latest session for principal when sessionId is omitted', async () => {});
it('resumePrincipal rejects when no session exists', async () => {});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: FAIL because lifecycle methods are not implemented.

**Step 3: Write minimal implementation**

Add:

```ts
import type { SpawnConfig } from './process-manager.js';

export interface PrincipalStartRequest {
  model?: string;
  modelProviderId?: string;
  modelId?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  permissionMode?: string;
}

export interface PrincipalResumeRequest extends PrincipalStartRequest {
  sessionId?: string;
}
```

Implement:

```ts
async startPrincipal(projectId: string, principalIdInput: string, input: PrincipalStartRequest = {}) {
  const project = this.requireProject(projectId);
  const principalId = normalizePrincipalId(principalIdInput);
  const existing = this.deps.processManager.findManagedAgentByPrincipal(principalId, projectId);
  if (existing) {
    return { status: 'already-running' as const, principalId, sessionId: existing.sessionId };
  }
  const config = this.buildSpawnConfig(project, principalId, input);
  const { spawnId } = this.deps.processManager.spawn(config);
  return { status: 'spawning' as const, spawnId, principalId };
}

async stopPrincipal(projectId: string, principalIdInput: string) {
  const principalId = normalizePrincipalId(principalIdInput);
  const stopped = this.deps.processManager.stopPrincipal(principalId, projectId);
  if (!stopped) {
    throw new PrincipalLifecycleError(404, `No managed agent found for principal ${principalId}`);
  }
  return { status: 'stopping' as const, principalId, sessionId: stopped.sessionId };
}

async resumePrincipal(projectId: string, principalIdInput: string, input: PrincipalResumeRequest = {}) {
  const project = this.requireProject(projectId);
  const principalId = normalizePrincipalId(principalIdInput);
  const existing = this.deps.processManager.findManagedAgentByPrincipal(principalId, projectId);
  if (existing) {
    return { status: 'already-running' as const, principalId, sessionId: existing.sessionId };
  }
  const session = this.findResumeSession(projectId, principalId, input.sessionId);
  if (!session.sessionJsonlPath) {
    throw new PrincipalLifecycleError(404, `No resumable session found for principal ${principalId}`);
  }
  const config = this.buildSpawnConfig(project, principalId, {
    ...input,
    model: input.model ?? session.model,
    modelProviderId: input.modelProviderId ?? session.modelProviderId,
    modelId: input.modelId ?? session.modelId,
    contextWindowTokens: input.contextWindowTokens ?? session.contextWindowTokens,
    maxOutputTokens: input.maxOutputTokens ?? session.maxOutputTokens,
  });
  config.sessionPath = session.sessionJsonlPath;
  const { spawnId } = this.deps.processManager.spawn(config);
  return { status: 'spawning' as const, spawnId, principalId, sessionId: session.sessionId };
}

private buildSpawnConfig(project: ProjectRecord, principalId: string, input: PrincipalStartRequest): SpawnConfig {
  const base = this.deps.processManager.getDefaultSpawnConfig();
  return {
    ...base,
    principalId,
    projectId: project.projectId,
    projectRoot: project.projectRoot,
    cwd: project.projectRoot,
    appfsMountRoot: project.mountRoot,
    model: input.model ?? base.model,
    modelProviderId: input.modelProviderId,
    modelId: input.modelId,
    contextWindowTokens: input.contextWindowTokens,
    maxOutputTokens: input.maxOutputTokens,
    permissionMode: input.permissionMode ?? base.permissionMode,
  };
}
```

`findResumeSession` should:

1. Call `agentRegistry.discoverProject(project.projectRoot)` if available.
2. Filter agents by `projectId` and `principalId`.
3. If `sessionId` is provided, use that exact session.
4. Otherwise sort by `startedAt` descending and pick the first with `sessionJsonlPath`.

**Step 4: Run test to verify it passes**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\principal-lifecycle.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/principal-lifecycle.ts dashboard/server/src/principal-lifecycle.test.ts
git commit -m "feat: orchestrate principal agent lifecycle"
```

## Task 7: Add Project-Scoped Principal Routes

**Files:**

- Modify: `dashboard/server/src/routes/principals.ts`
- Modify: `dashboard/server/src/index.ts`
- Test: `dashboard/server/src/routes/principals.test.ts`

**Step 1: Write the failing route tests**

Create `dashboard/server/src/routes/principals.test.ts`.

```ts
import { describe, it } from 'node:test';
import assert from 'node:assert';
import Fastify from 'fastify';
import { registerPrincipalsRoute } from './principals.js';

describe('Project scoped principals routes', () => {
  it('returns 404 for unknown project', async () => {
    const app = Fastify({ logger: false });
    registerPrincipalsRoute(app, fakeLifecycleService);
    const res = await app.inject({
      method: 'GET',
      url: '/api/projects/missing/principals',
    });
    assert.strictEqual(res.statusCode, 404);
    await app.close();
  });

  it('creates principal through lifecycle service', async () => {
    const app = Fastify({ logger: false });
    registerPrincipalsRoute(app, fakeLifecycleService);
    const res = await app.inject({
      method: 'POST',
      url: '/api/projects/project-a/principals',
      payload: { principalId: 'coder' },
    });
    assert.strictEqual(res.statusCode, 201);
    assert.match(res.payload, /"principal_id":"coder"/);
    await app.close();
  });
});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\routes\principals.test.ts
Pop-Location
```

Expected: FAIL because route signature and endpoints are not implemented.

**Step 3: Write minimal implementation**

Change route registration to accept a `PrincipalLifecycleService`.

```ts
export function registerPrincipalsRoute(
  app: FastifyInstance,
  lifecycle: PrincipalLifecycleService,
): void {
  app.get('/api/projects/:projectId/principals', async (request, reply) => {});
  app.post('/api/projects/:projectId/principals', async (request, reply) => {});
  app.delete('/api/projects/:projectId/principals/:principalId', async (request, reply) => {});
  app.post('/api/projects/:projectId/principals/:principalId/start', async (request, reply) => {});
  app.post('/api/projects/:projectId/principals/:principalId/stop', async (request, reply) => {});
  app.post('/api/projects/:projectId/principals/:principalId/resume', async (request, reply) => {});
}
```

Each handler should catch `PrincipalLifecycleError` and return `reply.status(err.statusCode).send({ error: err.message })`.

In `dashboard/server/src/index.ts`:

```ts
const principalLifecycle = new PrincipalLifecycleService({
  projectRegistry,
  agentRegistry: registry,
  processManager,
});
registerPrincipalsRoute(app, principalLifecycle);
```

Temporarily preserve legacy `GET /api/principals` by either:

1. Returning the first registered project's principal list; or
2. Keeping a compatibility function named `registerLegacyPrincipalsRoute`.

Prefer option 1 only if UI still depends on legacy endpoint. Otherwise migrate UI in Task 9 and remove legacy dependency.

**Step 4: Run route tests**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\routes\principals.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Run server test suite**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\*.test.ts src\routes\*.test.ts
Pop-Location
```

Expected: PASS.

**Step 6: Commit**

```powershell
git add dashboard/server/src/routes/principals.ts dashboard/server/src/routes/principals.test.ts dashboard/server/src/index.ts
git commit -m "feat: add project scoped principal lifecycle routes"
```

## Task 8: Keep Project Runtime Stop in Sync

**Files:**

- Modify: `dashboard/server/src/index.ts`
- Modify: `dashboard/server/src/process-manager.ts`
- Test: `dashboard/server/src/routes/projects.test.ts`

**Step 1: Write or update failing test**

Add a test proving project stop only stops managed agents for that project and removes managed sessions from project tracking when process exits.

```ts
it('project stop delegates to process manager for managed sessions only', async () => {
  // Use a fake processManager with stop calls recorded.
  // Ensure external session ids are not stopped.
});
```

**Step 2: Run test to verify it fails if behavior is incomplete**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\routes\projects.test.ts
Pop-Location
```

Expected: FAIL if current test coverage or cleanup is insufficient.

**Step 3: Minimal implementation**

Ensure `AgentProcessManager` process exit path detaches from `ProjectRegistry`:

```ts
if (managedAgent.sessionId && managedAgent.spawnConfig.projectId) {
  this.registry.projectRegistry.detachAgent(
    managedAgent.spawnConfig.projectId,
    managedAgent.sessionId,
  );
}
```

Ensure `session_started` path attaches:

```ts
if (managed.spawnConfig.projectId) {
  this.registry.projectRegistry.attachAgent(
    managed.spawnConfig.projectId,
    sessionId,
    'managed',
  );
}
```

**Step 4: Run tests**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\process-manager.test.ts src\routes\projects.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/process-manager.ts dashboard/server/src/process-manager.test.ts dashboard/server/src/routes/projects.test.ts
git commit -m "fix: keep project agent lifecycle tracking in sync"
```

## Task 9: Add Frontend Principal Lifecycle API Client

**Files:**

- Modify: `dashboard/src/api.ts` if present
- Otherwise create: `dashboard/src/principal-api.ts`
- Modify: `dashboard/src/types.ts`

**Step 1: Write or add type-only expectations**

If no frontend tests exist, add compile-time coverage by defining exported types and using them in components.

Types:

```ts
export interface PrincipalLifecycleInfo {
  principal_id: string;
  display_name?: string;
  description?: string | null;
  kind?: string;
  online: boolean;
  status: string;
  agent_status?: {
    state?: string;
    current_task_preview?: string | null;
    session_id?: string | null;
  } | null;
  pid?: number;
  sessionId?: string | null;
  model?: string;
  permissionMode?: string;
}
```

**Step 2: Implement API helpers**

```ts
export async function listProjectPrincipals(projectId: string): Promise<PrincipalListResponse> {
  const res = await fetch(`/api/projects/${encodeURIComponent(projectId)}/principals`);
  if (!res.ok) throw new Error(await errorText(res));
  return res.json();
}

export async function createProjectPrincipal(projectId: string, body: CreatePrincipalRequest) {}
export async function deleteProjectPrincipal(projectId: string, principalId: string) {}
export async function startProjectPrincipal(projectId: string, principalId: string, body: StartPrincipalRequest) {}
export async function stopProjectPrincipal(projectId: string, principalId: string) {}
export async function resumeProjectPrincipal(projectId: string, principalId: string, body?: ResumePrincipalRequest) {}
```

**Step 3: Run frontend build**

Run:

```powershell
npm --prefix dashboard run build
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add dashboard/src/types.ts dashboard/src/principal-api.ts
git commit -m "feat: add principal lifecycle frontend client"
```

## Task 10: Add Minimal Principal Lifecycle UI

**Files:**

- Modify: `dashboard/src/components/AgentSidebar.tsx`
- Modify: `dashboard/src/components/AgentItem.tsx`
- Modify: `dashboard/src/App.tsx`
- Modify: `dashboard/src/index.css`

**Step 1: Decide placement**

Use the existing agent sidebar first. Do not build a large new management surface.

Recommended UI:

- Existing session cards stay as session history.
- Add a compact "Principals" section above session cards.
- Each principal row has:
  - name
  - state badge
  - task preview one-line truncation
  - Start or Resume button when offline
  - Stop button when managed online
  - Delete button for non-default offline principals

**Step 2: Implement list loading**

In `App.tsx` or `AgentSidebar.tsx`, load principals when:

- selected project changes
- runtime starts/stops
- agent online/offline event arrives
- explicit refresh is clicked

**Step 3: Implement create principal**

Add a small inline form:

```tsx
<input value={newPrincipalId} placeholder="principal id" />
<button onClick={createPrincipal}>Create</button>
```

Validation:

- Empty id disables Create.
- Errors are shown in existing sidebar status area.

**Step 4: Wire Start/Stop/Resume/Delete**

Use the API helpers from Task 9.

Rules:

- Start uses current model picker/default spawn config.
- Resume calls `resumeProjectPrincipal(projectId, principalId)` with no session id for latest.
- Stop calls `stopProjectPrincipal(projectId, principalId)`.
- Delete is disabled for `default` and online principals.

**Step 5: Run frontend build**

Run:

```powershell
npm --prefix dashboard run build
```

Expected: PASS.

**Step 6: Manual smoke test**

Run dashboard and AppFS runtime, then verify:

1. Open project.
2. Start runtime.
3. Principals list shows `default`.
4. Create `coder`.
5. Start `coder`.
6. Stop `coder`.
7. Resume `coder`.
8. Delete an offline non-default principal.

**Step 7: Commit**

```powershell
git add dashboard/src/components/AgentSidebar.tsx dashboard/src/components/AgentItem.tsx dashboard/src/App.tsx dashboard/src/index.css
git commit -m "feat: manage principal agents from dashboard"
```

## Task 11: Refresh Existing Bootstrap Behavior

**Files:**

- Modify: `dashboard/server/src/process-manager.ts`
- Modify: `dashboard/server/src/routes/projects.ts`
- Test: `dashboard/server/src/routes/projects.test.ts`

**Step 1: Write failing test**

Add a test for bootstrap resume deduping by principal:

```ts
it('bootstrap does not resume multiple sessions for the same principal', async () => {
  // Arrange discovered sessions for principal coder.
  // Expect only the latest resumable session to spawn.
});
```

**Step 2: Run test to verify it fails**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\routes\projects.test.ts
Pop-Location
```

Expected: FAIL if `resumeProjectAgents` resumes every discovered session.

**Step 3: Minimal implementation**

Change `resumeProjectAgents(projectId)` to group by `principalId` and resume only the latest session per principal.

Pseudo-code:

```ts
const latestByPrincipal = new Map<string, AgentInfo>();
for (const agent of agents) {
  const principalId = agent.principalId || agent.name;
  const current = latestByPrincipal.get(principalId);
  if (!current || agent.startedAt > current.startedAt) {
    latestByPrincipal.set(principalId, agent);
  }
}
for (const agent of latestByPrincipal.values()) {
  // existing resume path
}
```

**Step 4: Run tests**

Run:

```powershell
Push-Location dashboard\server
npx tsx --test src\process-manager.test.ts src\routes\projects.test.ts
Pop-Location
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add dashboard/server/src/process-manager.ts dashboard/server/src/routes/projects.test.ts
git commit -m "fix: resume latest session per principal during bootstrap"
```

## Task 12: Backward Compatibility and Cleanup

**Files:**

- Modify: `dashboard/server/src/routes/principals.ts`
- Modify: `dashboard/src/components/AgentSidebar.tsx`
- Modify: docs if needed

**Step 1: Search for legacy principal endpoint usage**

Run:

```powershell
rg -n "/api/principals|api/principals" dashboard
```

Expected: No frontend usage after Task 10, or only compatibility route references.

**Step 2: Decide compatibility behavior**

If packaged UI no longer uses `/api/principals`, leave the route for one release as:

```ts
app.get('/api/principals', async (_request, reply) => {
  return reply.status(400).send({
    error: 'Use /api/projects/:projectId/principals',
  });
});
```

If older UI still needs it, return the first registered project's principal list.

**Step 3: Run build and tests**

Run:

```powershell
npm --prefix dashboard/server run build
Push-Location dashboard\server
npx tsx --test src\*.test.ts src\routes\*.test.ts
Pop-Location
npm --prefix dashboard run build
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add dashboard/server/src/routes/principals.ts dashboard/src/components/AgentSidebar.tsx
git commit -m "chore: prefer project scoped principal APIs"
```

## Task 13: Manual End-to-End Verification

**Files:**

- No code changes expected.

**Step 1: Start dashboard server/client**

Run:

```powershell
npm --prefix dashboard run dev
```

Expected: server and Vite client start successfully.

**Step 2: Open the desktop app or browser**

Open the dashboard and select:

```text
C:\Users\esp3j\rep\claude-code
```

**Step 3: Verify principal lifecycle**

1. Start runtime.
2. Confirm `default` appears in principal list.
3. Create `coder`.
4. Start `coder` with a non-default provider.
5. Send a message.
6. Stop `coder`.
7. Resume `coder`.
8. Confirm model provider and model config are preserved.
9. Try starting `coder` again while running.
10. Confirm the API returns `already-running`, not a duplicate process.

**Step 4: Verify AppFS status files**

Check:

```powershell
Get-Content C:\Users\esp3j\rep\claude-code\.appfs\_appfs\principals\status.res.json
Get-Content C:\Users\esp3j\rep\claude-code\.appfs\_appfs\principals\coder.res.json
```

Expected:

- `coder` appears.
- `agent_status.session_id` matches dashboard session.
- `agent_status.state` changes during work and returns idle/stopped after stop.

**Step 5: Verify delete behavior**

1. Stop `coder`.
2. Delete `coder`.
3. Confirm it disappears from `_appfs/principals.registry.json`.
4. Confirm deleting `default` returns an error.

## Task 14: Future Agent Tool Integration Design Note

**Files:**

- Create or modify: `docs/APPFS-principal-agent-lifecycle.md`

**Step 1: Document integration boundary**

Add:

```md
Future Claude Code-style Agent teammate mode should call the dashboard/server
PrincipalLifecycleService or an equivalent AppFS launcher bridge. It should not
write process-manager state directly and should not duplicate principal create,
resume, stop, or delete semantics.
```

**Step 2: Document deferred decisions**

Include:

- true attach takeover
- force delete
- external agent stop
- teammate `Agent(name=...)` mapping to principal create/start
- dashboard-managed vs runtime-managed launch authority

**Step 3: Commit**

```powershell
git add docs/APPFS-principal-agent-lifecycle.md
git commit -m "docs: define principal agent lifecycle boundary"
```

## Full Test Plan

Server:

```powershell
npm --prefix dashboard/server run build
Push-Location dashboard\server
npx tsx --test src\*.test.ts src\routes\*.test.ts
Pop-Location
```

Frontend:

```powershell
npm --prefix dashboard run build
```

Rust smoke, only if AppFS control-plane behavior is touched:

```powershell
cargo test --manifest-path appfs/cli/Cargo.toml principal
cargo check --manifest-path appfs/cli/Cargo.toml
cargo check --manifest-path appfs-agent/rust/Cargo.toml -p runtime -p rusty-claude-cli
```

Manual:

```powershell
npm --prefix dashboard run dev
```

Then use the dashboard to create, start, stop, resume, and delete principals in `C:\Users\esp3j\rep\claude-code`.

## Rollout Notes

1. Keep old session-centric routes working while adding principal-centric routes.
2. Migrate UI gradually; do not remove `/api/process/spawn` or `/api/agents/:sessionId/stop`.
3. Treat AppFS principal files as eventual consistency views; routes should return action accepted status without pretending AppFS has already materialized every file.
4. Polling for action materialization can be added later if the UI needs stronger confirmation.
5. Avoid introducing takeover until duplicate-start behavior is stable.

## Open Questions

1. Should `start` auto-create a missing principal, or return 404 until `create` is called? Recommended: return 404 after Task 5 adds reliable list/read behavior.
2. Should `delete` remove private app data under `.appfs/private/<principal>`? Recommended: no for v1.
3. Should project bootstrap resume all principals or only those previously dashboard-managed? Recommended: only latest known session per principal, preserving current managed/external distinction where possible.
4. Should future teammate agents be launched by dashboard server, appfs-agent itself, or a small local launcher bridge? Recommended: dashboard server first, then abstract if CLI-only use becomes important.
