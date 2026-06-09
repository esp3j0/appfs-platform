import { describe, it } from 'node:test';
import assert from 'node:assert';
import {
  AgentProcessManager,
  buildManagedAppfsAttachId,
  latestResumableAgentPerPrincipal,
  samePrincipalScope,
} from './process-manager.js';
import { AgentRegistry } from './agent-registry.js';
import { EventBus } from './event-bus.js';
import { ProjectRegistry } from './project-registry.js';
import type { AgentInfo } from './types.js';

describe('buildManagedAppfsAttachId', () => {
  it('returns a stable attach id scoped by principal id', () => {
    assert.strictEqual(buildManagedAppfsAttachId('default'), 'dashboard-default');
    assert.strictEqual(buildManagedAppfsAttachId('coder'), 'dashboard-coder');
  });

  it('sanitizes attach ids for AppFS lifecycle actions', () => {
    const attachId = buildManagedAppfsAttachId(' coder/main:1 ');

    assert.strictEqual(attachId, 'dashboard-coder-main-1');
    assert.match(attachId, /^[A-Za-z0-9_.-]+$/);
    assert.ok(attachId.length <= 160);
  });
});

describe('samePrincipalScope', () => {
  it('matches principal within a project scope', () => {
    assert.strictEqual(
      samePrincipalScope(
        { principalId: 'coder', projectId: 'project-a' },
        'coder',
        'project-a',
      ),
      true,
    );
    assert.strictEqual(
      samePrincipalScope(
        { principalId: 'coder', projectId: 'project-a' },
        'coder',
        'project-b',
      ),
      false,
    );
  });
});

describe('AgentProcessManager project tracking', () => {
  it('rejects a second managed process for the same project principal', () => {
    const projectRegistry = new ProjectRegistry();
    const project = projectRegistry.registerProject(process.cwd());
    const agentRegistry = new AgentRegistry(process.cwd(), projectRegistry);
    const manager = new AgentProcessManager(agentRegistry);

    try {
      const baseConfig = {
        cwd: project.projectRoot,
        principalId: 'coder',
        model: 'test-model',
        permissionMode: 'dangerous',
        appfsMountRoot: project.mountRoot,
        launchSpec: { kind: 'binary' as const, binaryPath: 'agent-bin' },
        env: {},
        projectId: project.projectId,
      };
      (manager as any).agents.set('spawn-existing', {
        process: fakeChildProcess(),
        sessionId: 'session-existing',
        spawnConfig: baseConfig,
        status: 'idle',
        currentRequestId: null,
        controlEndpoint: null,
        stdoutReader: fakeReader(),
        stderrReader: fakeReader(),
      });

      assert.throws(
        () => manager.spawn({ ...baseConfig }),
        /Managed agent already running for principal coder/,
      );
    } finally {
      EventBus.getInstance().shutdown();
    }
  });

  it('detaches managed sessions from their project when the process exits', () => {
    const projectRegistry = new ProjectRegistry();
    const project = projectRegistry.registerProject(process.cwd());
    const agentRegistry = new AgentRegistry(process.cwd(), projectRegistry);
    const manager = new AgentProcessManager(agentRegistry);
    const sessionId = 'session-detach-on-exit';

    try {
      const agentInfo: AgentInfo = {
        name: 'coder',
        principalId: 'coder',
        sessionId,
        model: 'test-model',
        pid: 123,
        startedAt: Date.now(),
        sessionJsonlPath: '',
        status: 'online',
        controlMode: 'managed',
        messageCount: 0,
        totalInputTokens: 0,
        totalOutputTokens: 0,
        projectId: project.projectId,
      };
      agentRegistry.registerAgent(agentInfo);
      assert.ok(project.managedAgentSessionIds.includes(sessionId));

      (manager as any).handleManagedExit('spawn-test', {
        process: fakeChildProcess(),
        sessionId,
        spawnConfig: {
          cwd: project.projectRoot,
          principalId: 'coder',
          model: 'test-model',
          permissionMode: 'dangerous',
          appfsMountRoot: project.mountRoot,
          launchSpec: { kind: 'binary', binaryPath: 'agent-bin' },
          env: {},
          projectId: project.projectId,
        },
        status: 'idle',
        currentRequestId: null,
        controlEndpoint: null,
        stdoutReader: fakeReader(),
        stderrReader: fakeReader(),
      }, 0, null);

      assert.ok(!project.agentSessionIds.includes(sessionId));
      assert.ok(!project.managedAgentSessionIds.includes(sessionId));
      assert.strictEqual(agentRegistry.getAgent(sessionId)?.status, 'offline');
    } finally {
      EventBus.getInstance().shutdown();
    }
  });

  it('waits for session_started before resolving a managed spawn', async () => {
    const projectRegistry = new ProjectRegistry();
    const project = projectRegistry.registerProject(process.cwd());
    const agentRegistry = new AgentRegistry(process.cwd(), projectRegistry);
    const manager = new AgentProcessManager(agentRegistry);
    const baseConfig = {
      cwd: project.projectRoot,
      principalId: 'coder',
      model: 'test-model',
      permissionMode: 'dangerous',
      appfsMountRoot: project.mountRoot,
      launchSpec: { kind: 'binary' as const, binaryPath: 'agent-bin' },
      env: {},
      projectId: project.projectId,
    };

    try {
      let resolveStarted!: () => void;
      const allowStarted = new Promise<void>((resolve) => {
        resolveStarted = resolve;
      });
      (manager as any).spawn = () => {
        (manager as any).agents.set('spawn-wait', {
          process: fakeChildProcess(),
          sessionId: null,
          spawnConfig: baseConfig,
          status: 'starting',
          currentRequestId: null,
          controlEndpoint: null,
          stdoutReader: fakeReader(),
          stderrReader: fakeReader(),
        });
        void allowStarted.then(() => {
          (manager as any).handleStdoutLine('spawn-wait', JSON.stringify({
            type: 'session_started',
            session_id: 'session-wait',
            principal_id: 'coder',
            session_path: '',
          }));
        });
        return { spawnId: 'spawn-wait' };
      };

      let resolved = false;
      const started = manager.spawnAndWaitStarted(baseConfig, 1000).then((result) => {
        resolved = true;
        return result;
      });
      await Promise.resolve();

      assert.strictEqual(resolved, false);
      resolveStarted();

      assert.deepStrictEqual(await started, {
        spawnId: 'spawn-wait',
        sessionId: 'session-wait',
      });
      assert.strictEqual(agentRegistry.getAgent('session-wait')?.status, 'online');
    } finally {
      EventBus.getInstance().shutdown();
    }
  });
});

describe('latestResumableAgentPerPrincipal', () => {
  it('keeps only the latest session per principal for bootstrap resume', () => {
    const agents = latestResumableAgentPerPrincipal([
      agentInfo('coder', 'session-old', 100),
      agentInfo('coder', 'session-new', 300),
      agentInfo('default', 'session-default', 200),
    ]);

    assert.deepStrictEqual(
      agents.map(agent => agent.sessionId),
      ['session-new', 'session-default'],
    );
  });

  it('excludes archived agents', () => {
    const agents = latestResumableAgentPerPrincipal([
      { ...agentInfo('coder', 'session-archived', 300), archived: true },
      agentInfo('coder', 'session-active', 100),
      agentInfo('default', 'session-default', 200),
    ]);

    // Results are sorted by startedAt descending
    assert.deepStrictEqual(
      agents.map(agent => agent.sessionId),
      ['session-default', 'session-active'],
    );
  });

  it('excludes agents without a principal id or name', () => {
    const agents = latestResumableAgentPerPrincipal([
      { ...agentInfo('coder', 'session-no-id', 100), principalId: '', name: '' },
      agentInfo('coder', 'session-with-id', 200),
    ]);

    assert.deepStrictEqual(
      agents.map(agent => agent.sessionId),
      ['session-with-id'],
    );
  });
});

function fakeReader(): { close: () => void } {
  return { close: () => undefined };
}

function fakeChildProcess(): any {
  return {
    pid: 123,
    stdout: { destroy: () => undefined },
    stderr: { destroy: () => undefined },
    stdin: {
      end: () => undefined,
      destroy: () => undefined,
    },
  };
}

function agentInfo(principalId: string, sessionId: string, startedAt: number): AgentInfo {
  return {
    name: principalId,
    principalId,
    sessionId,
    model: 'test-model',
    pid: 0,
    startedAt,
    sessionJsonlPath: `${sessionId}.jsonl`,
    status: 'offline',
    controlMode: 'external',
    messageCount: 0,
    totalInputTokens: 0,
    totalOutputTokens: 0,
  };
}
