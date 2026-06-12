import { describe, it } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
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

  it('injects AppFS and dashboard control environment for project managed agents', () => {
    const projectRegistry = new ProjectRegistry();
    const project = projectRegistry.registerProject(process.cwd());
    const agentRegistry = new AgentRegistry(process.cwd(), projectRegistry);
    const manager = new AgentProcessManager(agentRegistry, undefined, {
      apiOrigin: 'http://127.0.0.1:3100',
      controlToken: 'secret-token',
    });

    const env = (manager as any).buildEnvironment({
      cwd: project.projectRoot,
      principalId: 'coder',
      model: 'test-model',
      permissionMode: 'dangerous',
      appfsMountRoot: project.mountRoot,
      launchSpec: { kind: 'binary' as const, binaryPath: 'agent-bin' },
      env: { CUSTOM_ENV: '1' },
      projectId: project.projectId,
      teamName: 'alpha team',
      taskListId: 'alpha',
    });

    assert.strictEqual(env.APPFS_PRINCIPAL_ID, 'coder');
    assert.strictEqual(env.APPFS_ATTACH_ID, 'dashboard-coder');
    assert.strictEqual(env.APPFS_MOUNT_ROOT, path.resolve(project.mountRoot));
    assert.strictEqual(
      env.APPFS_RUNTIME_MANIFEST,
      path.join(path.resolve(project.mountRoot), '.well-known', 'appfs', 'runtime.json'),
    );
    assert.strictEqual(env.APPFS_DASHBOARD_API_ORIGIN, 'http://127.0.0.1:3100');
    assert.strictEqual(env.APPFS_DASHBOARD_PROJECT_ID, project.projectId);
    assert.strictEqual(env.APPFS_DASHBOARD_CONTROL_TOKEN, 'secret-token');
    assert.strictEqual(env.APPFS_TASK_LIST_ID, 'alpha');
    assert.strictEqual(env.CLAW_TASK_LIST_ID, 'alpha');
    assert.strictEqual(env.CLAUDE_CODE_TASK_LIST_ID, 'alpha');
    assert.strictEqual(env.APPFS_TEAM_NAME, 'alpha team');
    assert.strictEqual(env.CLAUDE_CODE_TEAM_NAME, 'alpha team');
    assert.strictEqual(env.CUSTOM_ENV, '1');
  });

  it('persists per-spawn agent stderr and stdout event summaries', () => {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-process-log-'));
    const previousLogDir = process.env.APPFS_LOG_DIR;
    process.env.APPFS_LOG_DIR = tempDir;

    const projectRegistry = new ProjectRegistry();
    const project = projectRegistry.registerProject(process.cwd());
    const agentRegistry = new AgentRegistry(process.cwd(), projectRegistry);
    const manager = new AgentProcessManager(agentRegistry);
    const spawnId = 'spawn-log-test';
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
      const managed = {
        process: fakeChildProcess(),
        sessionId: null,
        spawnConfig: baseConfig,
        status: 'starting',
        currentRequestId: null,
        controlEndpoint: null,
        stdoutReader: fakeReader(),
        stderrReader: fakeReader(),
        log: (manager as any).createAgentLog(spawnId, baseConfig),
      };
      (manager as any).agents.set(spawnId, managed);

      (manager as any).forwardStderrLine(
        spawnId,
        managed,
        'AppFS attach: checking identity...',
      );
      (manager as any).forwardStderrLine(
        spawnId,
        managed,
        'AppFS attach: warming up private apps...',
      );
      (manager as any).handleStdoutLine(spawnId, JSON.stringify({
        type: 'session_started',
        session_id: 'session-log-test',
        principal_id: 'coder',
        session_path: 'session-log-test.jsonl',
      }));

      const agentLogDir = path.join(tempDir, 'agents');
      const logFiles = fs.readdirSync(agentLogDir);
      assert.strictEqual(logFiles.length, 1);
      const logFile = logFiles[0];
      assert.ok(logFile);
      assert.match(logFile, /agent-.*-coder-spawn-log-test\.log/);
      const logContent = fs.readFileSync(path.join(agentLogDir, logFile), 'utf8');
      assert.match(logContent, /\[stderr\] AppFS attach: checking identity/);
      assert.match(logContent, /\[stderr\] AppFS attach: warming up private apps/);
      assert.match(logContent, /\[stdout-event\] type=session_started session=session-log-test principal=coder/);
    } finally {
      if (previousLogDir === undefined) {
        delete process.env.APPFS_LOG_DIR;
      } else {
        process.env.APPFS_LOG_DIR = previousLogDir;
      }
      fs.rmSync(tempDir, { recursive: true, force: true });
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
