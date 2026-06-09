import { describe, it } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {
  appendPrincipalAction,
  normalizePrincipalId,
  PrincipalLifecycleService,
  principalControlDir,
  readPrincipalViews,
} from './principal-lifecycle.js';
import { ProjectRegistry } from './project-registry.js';
import { samePrincipalScope, type SpawnConfig } from './process-manager.js';
import type { AgentInfo } from './types.js';

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
      /\/\.appfs\/_appfs\/principals$/,
    );
  });

  it('reads project-scoped principal registry and aggregate status', () => {
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'principal-lifecycle-'));
    const mountRoot = path.join(temp, '.appfs');
    const principalsDir = path.join(mountRoot, '_appfs', 'principals');
    fs.mkdirSync(principalsDir, { recursive: true });
    fs.writeFileSync(
      path.join(mountRoot, '_appfs', 'principals.registry.json'),
      JSON.stringify({
        version: 1,
        default_principal_id: 'default',
        principals: [{ principal_id: 'default', display_name: 'default', kind: 'agent' }],
      }),
    );
    fs.writeFileSync(
      path.join(principalsDir, 'status.res.json'),
      JSON.stringify({
        version: 1,
        principals: [{ principal_id: 'default', state: 'idle', session_id: 'session-a' }],
      }),
    );

    try {
      const views = readPrincipalViews(mountRoot);
      assert.strictEqual(views.default_principal_id, 'default');
      assert.strictEqual(views.principals[0]?.principal_id, 'default');
      assert.strictEqual(views.principals[0]?.agent_status?.state, 'idle');
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
  });

  it('appends create principal action JSONL', () => {
    const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'principal-action-'));
    const mountRoot = path.join(temp, '.appfs');

    try {
      appendPrincipalAction(mountRoot, 'create_principal.act', {
        principal_id: 'coder',
        display_name: 'coder',
        kind: 'agent',
      });

      const actionPath = path.join(mountRoot, '_appfs', 'principals', 'create_principal.act');
      const lines = fs.readFileSync(actionPath, 'utf8').trim().split(/\r?\n/);
      const parsed = JSON.parse(lines[0] ?? '{}');
      assert.strictEqual(parsed.principal_id, 'coder');
      assert.ok(parsed.client_token);
    } finally {
      fs.rmSync(temp, { recursive: true, force: true });
    }
  });
});

describe('PrincipalLifecycleService', () => {
  it('lists principals with project-scoped managed and discovered agent state', () => {
    const fixture = createFixture();

    try {
      writePrincipalRegistry(fixture.project.mountRoot, [
        { principal_id: 'coder', display_name: 'Coder', kind: 'agent' },
        { principal_id: 'reviewer', display_name: 'Reviewer', kind: 'agent' },
      ]);

      fixture.processManager.managedAgents = [
        {
          pid: 111,
          sessionId: 'session-coder-managed',
          status: 'idle',
          principalId: 'coder',
          projectId: fixture.project.projectId,
          model: 'managed-model',
          permissionMode: 'dangerous',
        },
        {
          pid: 222,
          sessionId: 'session-other-project',
          status: 'idle',
          principalId: 'reviewer',
          projectId: 'project-b',
          model: 'wrong-project-model',
          permissionMode: 'read-only',
        },
      ];
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'reviewer',
          sessionId: 'session-reviewer',
          projectId: fixture.project.projectId,
          model: 'reviewer-model',
          status: 'offline',
          startedAt: 200,
        }),
      ];

      const result = fixture.service.listPrincipals(fixture.project.projectId);

      const coder = result.principals.find((principal) => principal.principal_id === 'coder');
      const reviewer = result.principals.find((principal) => principal.principal_id === 'reviewer');
      assert.strictEqual(coder?.online, true);
      assert.strictEqual(coder?.sessionId, 'session-coder-managed');
      assert.strictEqual(coder?.permissionMode, 'dangerous');
      assert.strictEqual(reviewer?.online, false);
      assert.strictEqual(reviewer?.sessionId, 'session-reviewer');
      assert.strictEqual(reviewer?.model, 'reviewer-model');
    } finally {
      fixture.cleanup();
    }
  });

  it('creates a principal by appending an AppFS create action', async () => {
    const fixture = createFixture();

    try {
      const result = await fixture.service.createPrincipal(fixture.project.projectId, {
        principalId: 'coder',
        displayName: 'coder',
        description: 'Coding agent',
        kind: 'agent',
      });

      assert.strictEqual(result.status, 'created');
      const action = fs.readFileSync(
        path.join(fixture.project.mountRoot, '_appfs', 'principals', 'create_principal.act'),
        'utf8',
      );
      assert.match(action, /"principal_id":"coder"/);
      assert.match(action, /"description":"Coding agent"/);
    } finally {
      fixture.cleanup();
    }
  });

  it('deletes an offline default principal by appending an AppFS delete action', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'default',
          sessionId: 'session-default',
          projectId: fixture.project.projectId,
        }),
      ];

      const completed = completeNextDeleteAction(fixture.project.mountRoot, 'default');
      const result = await fixture.service.deletePrincipal(fixture.project.projectId, 'default');
      await completed;

      assert.deepStrictEqual(result, {
        status: 'deleted',
        principalId: 'default',
        archivedSessions: ['session-default'],
      });
      assert.strictEqual(fixture.agentRegistry.agents[0]?.archived, true);
      const action = fs.readFileSync(
        path.join(fixture.project.mountRoot, '_appfs', 'principals', 'delete_principal.act'),
        'utf8',
      );
      assert.match(action, /"principal_id":"default"/);
    } finally {
      fixture.cleanup();
    }
  });

  it('deletes an offline principal by appending an AppFS delete action', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-coder',
          projectId: fixture.project.projectId,
        }),
      ];

      const completed = completeNextDeleteAction(fixture.project.mountRoot, 'coder');
      const result = await fixture.service.deletePrincipal(fixture.project.projectId, 'coder');
      await completed;

      assert.deepStrictEqual(result, {
        status: 'deleted',
        principalId: 'coder',
        archivedSessions: ['session-coder'],
      });
      assert.strictEqual(fixture.agentRegistry.agents[0]?.archived, true);
      const action = fs.readFileSync(
        path.join(fixture.project.mountRoot, '_appfs', 'principals', 'delete_principal.act'),
        'utf8',
      );
      assert.match(action, /"principal_id":"coder"/);
    } finally {
      fixture.cleanup();
    }
  });

  it('does not archive sessions when AppFS rejects delete_principal', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-coder',
          projectId: fixture.project.projectId,
        }),
      ];

      const failed = completeNextDeleteAction(fixture.project.mountRoot, 'coder', {
        type: 'action.failed',
        error: {
          code: 'APPFS_DELETE_FAILED',
          message: 'delete failed in AppFS',
        },
      });

      await assert.rejects(
        () => fixture.service.deletePrincipal(fixture.project.projectId, 'coder'),
        /APPFS_DELETE_FAILED: delete failed in AppFS/,
      );
      await failed;
      assert.strictEqual(fixture.agentRegistry.agents[0]?.archived, undefined);
    } finally {
      fixture.cleanup();
    }
  });

  it('force deletes a stale AppFS attach after managed and registry agents are offline', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-coder',
          projectId: fixture.project.projectId,
        }),
      ];
      writePrincipalRegistry(fixture.project.mountRoot, [{
        principal_id: 'coder',
        active_attach_count: 1,
        active_attaches: [{
          attach_id: 'dashboard-coder',
          last_seen_at: new Date(Date.now() - 120_000).toISOString(),
        }],
        presence: 'online',
      }]);

      const completed = completeNextDeleteAction(fixture.project.mountRoot, 'coder');
      const result = await fixture.service.deletePrincipal(fixture.project.projectId, 'coder');
      await completed;

      assert.deepStrictEqual(result, {
        status: 'deleted',
        principalId: 'coder',
        archivedSessions: ['session-coder'],
      });
      const action = fs.readFileSync(
        path.join(fixture.project.mountRoot, '_appfs', 'principals', 'delete_principal.act'),
        'utf8',
      );
      assert.match(action, /"principal_id":"coder"/);
      assert.match(action, /"force":true/);
    } finally {
      fixture.cleanup();
    }
  });

  it('rejects deleting a fresh AppFS attach', async () => {
    const fixture = createFixture();

    try {
      writePrincipalRegistry(fixture.project.mountRoot, [{
        principal_id: 'coder',
        active_attach_count: 1,
        active_attaches: [{
          attach_id: 'dashboard-coder',
          last_seen_at: new Date().toISOString(),
        }],
        presence: 'online',
      }]);

      await assert.rejects(
        () => fixture.service.deletePrincipal(fixture.project.projectId, 'coder'),
        /active AppFS attach/,
      );
      assert.strictEqual(
        fs.existsSync(path.join(fixture.project.mountRoot, '_appfs', 'principals', 'delete_principal.act')),
        false,
      );
    } finally {
      fixture.cleanup();
    }
  });

  it('rejects deleting an online principal', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-online',
          projectId: fixture.project.projectId,
          status: 'online',
        }),
      ];

      await assert.rejects(
        () => fixture.service.deletePrincipal(fixture.project.projectId, 'coder'),
        /online agent/,
      );
    } finally {
      fixture.cleanup();
    }
  });

  it('startPrincipal returns existing managed agent instead of spawning duplicate', async () => {
    const fixture = createFixture();

    try {
      fixture.processManager.managedAgents = [
        {
          pid: 123,
          sessionId: 'session-existing',
          status: 'busy',
          principalId: 'coder',
          projectId: fixture.project.projectId,
          model: 'managed-model',
          permissionMode: 'dangerous',
        },
      ];

      const result = await fixture.service.startPrincipal(fixture.project.projectId, 'coder');

      assert.deepStrictEqual(result, {
        status: 'already-running',
        principalId: 'coder',
        sessionId: 'session-existing',
      });
      assert.deepStrictEqual(fixture.processManager.spawned, []);
    } finally {
      fixture.cleanup();
    }
  });

  it('startPrincipal spawns new agent with project-scoped config', async () => {
    const fixture = createFixture();

    try {
      const result = await fixture.service.startPrincipal(fixture.project.projectId, 'coder', {
        model: 'claude-opus-test',
        modelProviderId: 'anthropic-default',
        modelId: 'opus-test',
        contextWindowTokens: 200000,
        maxOutputTokens: 8192,
        permissionMode: 'workspace-write',
      });

      assert.deepStrictEqual(result, {
        status: 'spawning',
        spawnId: 'spawn-1',
        principalId: 'coder',
      });
      assert.strictEqual(fixture.processManager.spawned.length, 1);
      const config = fixture.processManager.spawned[0];
      assert.strictEqual(config?.principalId, 'coder');
      assert.strictEqual(config?.projectId, fixture.project.projectId);
      assert.strictEqual(config?.projectRoot, fixture.project.projectRoot);
      assert.strictEqual(config?.cwd, fixture.project.projectRoot);
      assert.strictEqual(config?.appfsMountRoot, fixture.project.mountRoot);
      assert.strictEqual(config?.model, 'claude-opus-test');
      assert.strictEqual(config?.modelProviderId, 'anthropic-default');
      assert.strictEqual(config?.modelId, 'opus-test');
      assert.strictEqual(config?.contextWindowTokens, 200000);
      assert.strictEqual(config?.maxOutputTokens, 8192);
      assert.strictEqual(config?.permissionMode, 'workspace-write');
      assert.deepStrictEqual(config?.launchSpec, fixture.processManager.defaultConfig.launchSpec);
    } finally {
      fixture.cleanup();
    }
  });

  it('stopPrincipal stops managed agent and detaches AppFS attach by principal id', async () => {
    const fixture = createFixture();

    try {
      fixture.processManager.managedAgents = [
        {
          pid: 123,
          sessionId: 'session-stop',
          status: 'idle',
          principalId: 'coder',
          projectId: fixture.project.projectId,
          model: 'managed-model',
          permissionMode: 'dangerous',
        },
      ];
      writePrincipalRegistry(fixture.project.mountRoot, [{
        principal_id: 'coder',
        active_attach_count: 1,
        active_attaches: [{
          attach_id: 'dashboard-coder',
          last_seen_at: new Date().toISOString(),
        }],
      }]);

      const completed = completeNextDetachAction(fixture.project.mountRoot, 'coder');
      const result = await fixture.service.stopPrincipal(fixture.project.projectId, 'coder');
      await completed;

      assert.deepStrictEqual(result, {
        status: 'stopped',
        principalId: 'coder',
        sessionId: 'session-stop',
        detach: {
          detached: true,
          attachId: 'dashboard-coder',
          reason: 'detached',
        },
      });
      assert.deepStrictEqual(fixture.processManager.stopped, [
        { principalId: 'coder', projectId: fixture.project.projectId },
      ]);
      const action = fs.readFileSync(
        path.join(fixture.project.mountRoot, '_appfs', 'principals', 'detach_principal.act'),
        'utf8',
      );
      assert.match(action, /"principal_id":"coder"/);
      assert.match(action, /"attach_id":"dashboard-coder"/);
      assert.match(action, /"reason":"dashboard_stop"/);
    } finally {
      fixture.cleanup();
    }
  });

  it('resumePrincipal chooses latest session for principal when sessionId is omitted', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-old',
          projectId: fixture.project.projectId,
          startedAt: 100,
          sessionJsonlPath: path.join(fixture.tempDir, 'old.jsonl'),
          model: 'old-model',
        }),
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-new',
          projectId: fixture.project.projectId,
          startedAt: 300,
          sessionJsonlPath: path.join(fixture.tempDir, 'new.jsonl'),
          model: 'new-model',
          modelProviderId: 'provider-new',
          modelId: 'model-new',
          contextWindowTokens: 123000,
          maxOutputTokens: 4560,
        }),
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-other-project',
          projectId: 'project-b',
          startedAt: 500,
          sessionJsonlPath: path.join(fixture.tempDir, 'other.jsonl'),
          model: 'wrong-model',
        }),
      ];

      const result = await fixture.service.resumePrincipal(fixture.project.projectId, 'coder');

      assert.deepStrictEqual(result, {
        status: 'spawning',
        spawnId: 'spawn-1',
        principalId: 'coder',
        sessionId: 'session-new',
      });
      assert.deepStrictEqual(fixture.agentRegistry.discoverProjectCalls, [
        fixture.project.projectRoot,
      ]);
      const config = fixture.processManager.spawned[0];
      assert.strictEqual(config?.sessionPath, path.join(fixture.tempDir, 'new.jsonl'));
      assert.strictEqual(config?.model, 'new-model');
      assert.strictEqual(config?.modelProviderId, 'provider-new');
      assert.strictEqual(config?.modelId, 'model-new');
      assert.strictEqual(config?.contextWindowTokens, 123000);
      assert.strictEqual(config?.maxOutputTokens, 4560);
    } finally {
      fixture.cleanup();
    }
  });

  it('resumePrincipal uses a caller-provided session id when present', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-old',
          projectId: fixture.project.projectId,
          startedAt: 100,
          sessionJsonlPath: path.join(fixture.tempDir, 'old.jsonl'),
          model: 'old-model',
        }),
        agentInfo({
          principalId: 'coder',
          sessionId: 'session-new',
          projectId: fixture.project.projectId,
          startedAt: 300,
          sessionJsonlPath: path.join(fixture.tempDir, 'new.jsonl'),
          model: 'new-model',
        }),
      ];

      const result = await fixture.service.resumePrincipal(fixture.project.projectId, 'coder', {
        sessionId: 'session-old',
      });

      assert.strictEqual(result.sessionId, 'session-old');
      assert.strictEqual(fixture.processManager.spawned[0]?.sessionPath, path.join(fixture.tempDir, 'old.jsonl'));
    } finally {
      fixture.cleanup();
    }
  });

  it('resumePrincipal rejects when no session exists', async () => {
    const fixture = createFixture();

    try {
      await assert.rejects(
        () => fixture.service.resumePrincipal(fixture.project.projectId, 'coder'),
        /No resumable session found/,
      );
      assert.deepStrictEqual(fixture.processManager.spawned, []);
    } finally {
      fixture.cleanup();
    }
  });

  it('resumeProjectPrincipals waits for each agent to publish session_started before starting the next', async () => {
    const fixture = createFixture();

    try {
      fixture.agentRegistry.agents = [
        agentInfo({
          principalId: 'coder-new',
          sessionId: 'session-new',
          projectId: fixture.project.projectId,
          startedAt: 300,
          sessionJsonlPath: path.join(fixture.tempDir, 'session-new.jsonl'),
        }),
        agentInfo({
          principalId: 'coder-old',
          sessionId: 'session-old',
          projectId: fixture.project.projectId,
          startedAt: 100,
          sessionJsonlPath: path.join(fixture.tempDir, 'session-old.jsonl'),
        }),
      ];

      let releaseFirst!: () => void;
      const firstStarted = new Promise<void>((resolve) => {
        releaseFirst = resolve;
      });
      fixture.processManager.spawnAndWaitStartedHandler = async (_config, spawnId) => {
        if (spawnId === 'spawn-1') {
          await firstStarted;
        }
        return { spawnId, sessionId: `started-${spawnId}` };
      };

      const resume = fixture.service.resumeProjectPrincipals(fixture.project.projectId);
      await Promise.resolve();

      assert.strictEqual(fixture.processManager.spawned.length, 1);
      assert.strictEqual(fixture.processManager.spawned[0]?.principalId, 'coder-new');

      releaseFirst();
      const result = await resume;

      assert.strictEqual(fixture.processManager.spawned.length, 2);
      assert.deepStrictEqual(
        fixture.processManager.spawned.map((config) => config.principalId),
        ['coder-new', 'coder-old'],
      );
      assert.deepStrictEqual(result.resumed, [
        { sessionId: 'session-new', spawnId: 'spawn-1' },
        { sessionId: 'session-old', spawnId: 'spawn-2' },
      ]);
      assert.deepStrictEqual(result.errors, []);
    } finally {
      fixture.cleanup();
    }
  });
});

function createFixture() {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'principal-lifecycle-service-'));
  const projectRegistry = new ProjectRegistry();
  const project = projectRegistry.registerProject(tempDir);
  const agentRegistry = new FakeAgentRegistry();
  const processManager = new FakeProcessManager();
  const service = new PrincipalLifecycleService({
    projectRegistry,
    agentRegistry,
    processManager,
  });

  return {
    tempDir,
    project,
    agentRegistry,
    processManager,
    service,
    cleanup: () => fs.rmSync(tempDir, { recursive: true, force: true }),
  };
}

function writePrincipalRegistry(
  mountRoot: string,
  principals: Array<{
    principal_id: string;
    display_name?: string;
    kind?: string;
    presence?: string;
    active_attach_count?: number;
    active_attaches?: Array<{ attach_id?: string; last_seen_at?: string }>;
  }>,
): void {
  fs.mkdirSync(path.join(mountRoot, '_appfs'), { recursive: true });
  fs.writeFileSync(
    path.join(mountRoot, '_appfs', 'principals.registry.json'),
    JSON.stringify({
      version: 1,
      default_principal_id: 'default',
      principals,
    }),
  );
}

function completeNextDeleteAction(
  mountRoot: string,
  principalId: string,
  outcome: {
    type: 'action.completed' | 'action.failed';
    error?: { code: string; message: string };
  } = { type: 'action.completed' },
): Promise<void> {
  const actionPath = path.join(mountRoot, '_appfs', 'principals', 'delete_principal.act');
  return new Promise((resolve, reject) => {
    let attempts = 0;
    const timer = setInterval(() => {
      attempts += 1;
      try {
        if (fs.existsSync(actionPath)) {
          const action = findActionForPrincipal(actionPath, principalId);
          if (action?.client_token) {
            clearInterval(timer);
            writeControlEvent(mountRoot, {
              path: '/_appfs/principals/delete_principal.act',
              type: outcome.type,
              client_token: action.client_token,
              content: outcome.type === 'action.completed'
                ? { principal_event: 'principal.deleted', principal_id: principalId }
                : undefined,
              error: outcome.error,
            });
            resolve();
            return;
          }
        }
        if (attempts > 100) {
          clearInterval(timer);
          reject(new Error(`delete action for ${principalId} was not appended`));
        }
      } catch (err) {
        clearInterval(timer);
        reject(err);
      }
    }, 10);
  });
}

function completeNextDetachAction(
  mountRoot: string,
  principalId: string,
  outcome: {
    type: 'action.completed' | 'action.failed';
    error?: { code: string; message: string };
  } = { type: 'action.completed' },
): Promise<void> {
  const actionPath = path.join(mountRoot, '_appfs', 'principals', 'detach_principal.act');
  return new Promise((resolve, reject) => {
    let attempts = 0;
    const timer = setInterval(() => {
      attempts += 1;
      try {
        if (fs.existsSync(actionPath)) {
          const action = findActionForPrincipal(actionPath, principalId);
          if (action?.client_token) {
            clearInterval(timer);
            writeControlEvent(mountRoot, {
              path: '/_appfs/principals/detach_principal.act',
              type: outcome.type,
              client_token: action.client_token,
              content: outcome.type === 'action.completed'
                ? {
                    principal_event: 'principal.detached',
                    principal_id: principalId,
                    attach_id: action.attach_id,
                  }
                : undefined,
              error: outcome.error,
            });
            resolve();
            return;
          }
        }
        if (attempts > 100) {
          clearInterval(timer);
          reject(new Error(`detach action for ${principalId} was not appended`));
        }
      } catch (err) {
        clearInterval(timer);
        reject(err);
      }
    }, 10);
  });
}

function findActionForPrincipal(
  actionPath: string,
  principalId: string,
): { client_token?: string; attach_id?: string } | null {
  const lines = fs.readFileSync(actionPath, 'utf8').trim().split(/\r?\n/).reverse();
  for (const line of lines) {
    if (!line.trim()) {
      continue;
    }
    const parsed = JSON.parse(line) as { principal_id?: string; client_token?: string; attach_id?: string };
    if (parsed.principal_id === principalId) {
      return parsed;
    }
  }
  return null;
}

function writeControlEvent(
  mountRoot: string,
  event: Record<string, unknown>,
): void {
  const streamDir = path.join(mountRoot, '_appfs', '_stream');
  fs.mkdirSync(streamDir, { recursive: true });
  fs.appendFileSync(
    path.join(streamDir, 'events.evt.jsonl'),
    `${JSON.stringify({
      seq: Date.now(),
      event_id: `evt-${Date.now()}`,
      ts: new Date().toISOString(),
      app: '_appfs',
      session_id: 'runtime-control',
      request_id: `test-${Date.now()}`,
      ...event,
    })}\n`,
    'utf8',
  );
}

class FakeAgentRegistry {
  agents: AgentInfo[] = [];
  discoverProjectCalls: string[] = [];

  discoverProject(projectRoot: string): void {
    this.discoverProjectCalls.push(projectRoot);
  }

  getAgents(): AgentInfo[] {
    return this.agents;
  }

  archiveSessionsForPrincipal(
    principalId: string,
    projectId?: string,
    reason = 'principal_deleted',
  ): AgentInfo[] {
    const archived: AgentInfo[] = [];
    const archivedAt = Date.now();
    this.agents = this.agents.map((agent) => {
      if ((agent.principalId || agent.name) !== principalId || (projectId && agent.projectId !== projectId) || agent.archived) {
        return agent;
      }
      const updated = {
        ...agent,
        archived: true,
        archivedAt,
        archivedReason: reason,
      };
      archived.push(updated);
      return updated;
    });
    return archived;
  }
}

class FakeProcessManager {
  defaultConfig: SpawnConfig = {
    cwd: 'base-cwd',
    principalId: 'default',
    model: 'base-model',
    permissionMode: 'dangerous',
    appfsMountRoot: 'base-mount',
    appfsIdleWake: true,
    env: { BASE: '1' },
    launchSpec: { kind: 'binary', binaryPath: 'agent-bin' },
  };
  managedAgents: Array<{
    pid?: number;
    sessionId: string | null;
    status: 'starting' | 'idle' | 'busy';
    principalId: string;
    projectId?: string;
    model: string;
    permissionMode: string;
  }> = [];
  spawned: SpawnConfig[] = [];
  stopped: Array<{ principalId: string; projectId?: string }> = [];
  spawnAndWaitStartedHandler?: (config: SpawnConfig, spawnId: string) => Promise<{ spawnId: string; sessionId: string }>;

  getDefaultSpawnConfig(): SpawnConfig {
    return this.defaultConfig;
  }

  getManagedAgents() {
    return this.managedAgents;
  }

  findManagedAgentByPrincipal(principalId: string, projectId?: string) {
    return this.managedAgents.find((agent) => samePrincipalScope(agent, principalId, projectId)) ?? null;
  }

  spawn(config: SpawnConfig): { spawnId: string } {
    this.spawned.push(config);
    return { spawnId: `spawn-${this.spawned.length}` };
  }

  async spawnAndWaitStarted(config: SpawnConfig): Promise<{ spawnId: string; sessionId: string }> {
    const { spawnId } = this.spawn(config);
    if (this.spawnAndWaitStartedHandler) {
      return this.spawnAndWaitStartedHandler(config, spawnId);
    }
    return {
      spawnId,
      sessionId: config.sessionPath ? path.basename(config.sessionPath, '.jsonl') : spawnId,
    };
  }

  async stopPrincipal(principalId: string, projectId?: string): Promise<{ sessionId: string | null } | null> {
    this.stopped.push({ principalId, projectId });
    const managed = this.findManagedAgentByPrincipal(principalId, projectId);
    return managed ? { sessionId: managed.sessionId } : null;
  }
}

function agentInfo(overrides: Partial<AgentInfo> & { principalId: string; sessionId: string }): AgentInfo {
  const { principalId, sessionId, ...rest } = overrides;
  return {
    name: principalId,
    principalId,
    sessionId,
    model: 'test-model',
    pid: 0,
    startedAt: 1,
    sessionJsonlPath: path.join(os.tmpdir(), `${sessionId}.jsonl`),
    status: 'offline',
    controlMode: 'external',
    messageCount: 0,
    totalInputTokens: 0,
    totalOutputTokens: 0,
    ...rest,
  };
}
