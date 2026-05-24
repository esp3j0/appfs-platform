import { describe, it, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { ProjectRegistry, walkProjectDirectory, checkMountpointConflict } from './project-registry.js';
import { AgentRegistry } from './agent-registry.js';
import type { AgentInfo } from './types.js';
import Fastify from 'fastify';
import { registerProjectsRoute } from './routes/projects.js';
import { resolveProjectScopedSpawnConfig, type SpawnConfig } from './process-manager.js';

describe('ProjectRegistry & Path Model (P0)', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'appfs-test-project-'));
  });

  afterEach(() => {
    fs.rmSync(tempDir, { recursive: true, force: true });
  });

  it('should register a project and resolve correct paths', () => {
    const registry = new ProjectRegistry();
    const record = registry.registerProject(tempDir);

    assert.ok(record.projectId);
    assert.strictEqual(record.projectRoot, path.resolve(tempDir));
    assert.strictEqual(record.composeFilePath, path.join(path.resolve(tempDir), '.appfs-compose.yaml'));
    assert.strictEqual(record.mountRoot, path.join(path.resolve(tempDir), '.appfs'));
    assert.strictEqual(record.status, 'stopped');
    assert.deepStrictEqual(record.managedAgentSessionIds, []);
  });

  it('should have a stable projectId based on projectRoot', () => {
    const registry = new ProjectRegistry();
    const record1 = registry.registerProject(tempDir);
    
    // Clear and re-register same path
    registry.clear();
    const record2 = registry.registerProject(tempDir);
    assert.strictEqual(record1.projectId, record2.projectId);

    // Register a different path
    const tempDir2 = fs.mkdtempSync(path.join(os.tmpdir(), 'appfs-test-project2-'));
    try {
      const record3 = registry.registerProject(tempDir2);
      assert.notStrictEqual(record1.projectId, record3.projectId);
    } finally {
      fs.rmSync(tempDir2, { recursive: true, force: true });
    }
  });

  it('should walk project directory while strictly ignoring .appfs and .claw', () => {
    // Create some project files
    fs.writeFileSync(path.join(tempDir, 'main.ts'), 'console.log("hello");');
    fs.mkdirSync(path.join(tempDir, 'src'));
    fs.writeFileSync(path.join(tempDir, 'src', 'index.ts'), 'export {}');

    // Create .appfs (mount) directory with a file
    fs.mkdirSync(path.join(tempDir, '.appfs'));
    fs.writeFileSync(path.join(tempDir, '.appfs', 'something-private.json'), '{}');

    // Create .claw session directory with files
    fs.mkdirSync(path.join(tempDir, '.claw'));
    fs.writeFileSync(path.join(tempDir, '.claw', 'session-1.jsonl'), '{}');

    const files = walkProjectDirectory(tempDir).map(p => path.relative(tempDir, p).replace(/\\/g, '/'));
    
    // Should contain normal project files
    assert.ok(files.includes('main.ts'));
    assert.ok(files.includes('src/index.ts'));

    // Should NOT contain anything inside .appfs or .claw
    assert.ok(!files.includes('.appfs/something-private.json'));
    assert.ok(!files.includes('.claw/session-1.jsonl'));
    assert.ok(!files.includes('.appfs'));
    assert.ok(!files.includes('.claw'));
  });

  it('should NOT throw on registerProject if .appfs is non-empty or is a file, but checkMountpointConflict should throw', () => {
    const registry = new ProjectRegistry();

    // 1. Create a non-empty .appfs directory
    const appfsPath = path.join(tempDir, '.appfs');
    fs.mkdirSync(appfsPath);
    fs.writeFileSync(path.join(appfsPath, 'some-file.txt'), 'content');

    // registerProject should succeed
    let record;
    assert.doesNotThrow(() => {
      record = registry.registerProject(tempDir);
    });
    assert.ok(record);

    // checkMountpointConflict should throw on non-empty .appfs
    assert.throws(() => {
      checkMountpointConflict(appfsPath);
    }, /Conflict detected/);

    // 2. Make .appfs a file instead of a directory
    fs.rmSync(appfsPath, { recursive: true, force: true });
    fs.writeFileSync(appfsPath, 'I am a file');

    // registerProject should still succeed
    assert.doesNotThrow(() => {
      registry.registerProject(tempDir);
    });

    // checkMountpointConflict should throw on file .appfs
    assert.throws(() => {
      checkMountpointConflict(appfsPath);
    }, /Conflict detected/);
  });

  it('should be idempotent and not throw conflict on re-register even if .appfs is non-empty', () => {
    const registry = new ProjectRegistry();
    const record = registry.registerProject(tempDir);

    // Make .appfs non-empty to simulate active mount
    const appfsPath = path.join(tempDir, '.appfs');
    if (!fs.existsSync(appfsPath)) {
      fs.mkdirSync(appfsPath);
    }
    fs.writeFileSync(path.join(appfsPath, 'mounted-file.txt'), 'some content');

    // Re-register should succeed and return same project without throwing conflict
    let reRecord;
    assert.doesNotThrow(() => {
      reRecord = registry.registerProject(tempDir);
    });
    assert.strictEqual(reRecord, record);
  });



  it('should list projects and handle opening via Fastify routes', async () => {
    const registry = new ProjectRegistry();
    const app = Fastify({ logger: false });
    try {
      const dummyController = {
        start: async () => { throw new Error('Not implemented'); },
        stop: async () => { throw new Error('Not implemented'); },
        status: () => undefined,
      };
      registerProjectsRoute(app, registry, dummyController);

      // Initial list
      let res = await app.inject({
        method: 'GET',
        url: '/api/projects'
      });
      assert.strictEqual(res.statusCode, 200);
      let data = JSON.parse(res.payload);
      assert.deepStrictEqual(data.projects, []);

      // Register project
      res = await app.inject({
        method: 'POST',
        url: '/api/projects/open',
        payload: { projectRoot: tempDir }
      });
      assert.strictEqual(res.statusCode, 200);
      const record = JSON.parse(res.payload);
      assert.strictEqual(record.projectRoot, path.resolve(tempDir));

      // List after registering
      res = await app.inject({
        method: 'GET',
        url: '/api/projects'
      });
      assert.strictEqual(res.statusCode, 200);
      data = JSON.parse(res.payload);
      assert.strictEqual(data.projects.length, 1);
      assert.strictEqual(data.projects[0].projectId, record.projectId);

      // Get specific project status
      res = await app.inject({
        method: 'GET',
        url: `/api/projects/${record.projectId}`
      });
      assert.strictEqual(res.statusCode, 200);
      const statusRecord = JSON.parse(res.payload);
      assert.strictEqual(statusRecord.projectRoot, path.resolve(tempDir));

      // Try to open non-existent directory
      res = await app.inject({
        method: 'POST',
        url: '/api/projects/open',
        payload: { projectRoot: '/non-existent-dir-abc-123' }
      });
      assert.strictEqual(res.statusCode, 400);
    } finally {
      await app.close();
    }
  });

  it('should attach managed and external agents correctly to projectSession arrays', () => {
    const projectRegistry = new ProjectRegistry();
    const agentRegistry = new AgentRegistry(tempDir, projectRegistry);

    const project = projectRegistry.registerProject(tempDir);
    const sessionManaged = 'session-managed';
    const sessionExternal = 'session-external';

    // 1. Register a managed agent
    const managedInfo: AgentInfo = {
      name: 'managed-1',
      principalId: 'p-1',
      sessionId: sessionManaged,
      model: 'test',
      pid: 101,
      startedAt: Date.now(),
      sessionJsonlPath: '',
      status: 'online',
      controlMode: 'managed',
      messageCount: 0,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      projectId: project.projectId,
    };
    agentRegistry.registerAgent(managedInfo);

    // Assert managed is in both arrays
    const record = projectRegistry.getProject(project.projectId)!;
    assert.ok(record.agentSessionIds.includes(sessionManaged));
    assert.ok(record.managedAgentSessionIds.includes(sessionManaged));

    // 2. Register an external agent
    const externalInfo: AgentInfo = {
      name: 'external-1',
      principalId: 'p-2',
      sessionId: sessionExternal,
      model: 'test',
      pid: 102,
      startedAt: Date.now(),
      sessionJsonlPath: '',
      status: 'offline',
      controlMode: 'external',
      messageCount: 0,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      projectId: project.projectId,
    };
    agentRegistry.registerAgent(externalInfo);

    // Assert external is in agentSessionIds but NOT managedAgentSessionIds
    assert.ok(record.agentSessionIds.includes(sessionExternal));
    assert.ok(!record.managedAgentSessionIds.includes(sessionExternal));

    // 3. attachAgent returns false if project not found
    const attached = projectRegistry.attachAgent('non-existent-pid', 'session-xyz', 'managed');
    assert.strictEqual(attached, false);
  });

  it('should fallback and infer project when non-existent projectId is passed but path matches', () => {
    const projectRegistry = new ProjectRegistry();
    const agentRegistry = new AgentRegistry(tempDir, projectRegistry);

    const project = projectRegistry.registerProject(tempDir);
    const sessionJsonlPath = path.join(tempDir, '.claw', 'sessions', 'fingerprint-x', 'session-y.jsonl');

    const agentInfo: AgentInfo = {
      name: 'fallback-agent',
      principalId: 'p-3',
      sessionId: 'session-y',
      model: 'test',
      pid: 103,
      startedAt: Date.now(),
      sessionJsonlPath,
      status: 'online',
      controlMode: 'managed',
      messageCount: 0,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      projectId: 'incorrect-project-id', // Incorrect/non-existent projectId
    };

    agentRegistry.registerAgent(agentInfo);

    // Should have inferred the correct project based on session path fallback
    const registered = agentRegistry.getAgent('session-y')!;
    assert.strictEqual(registered.projectId, project.projectId);
    assert.strictEqual(registered.projectRoot, project.projectRoot);
    assert.ok(project.agentSessionIds.includes('session-y'));
    assert.ok(project.managedAgentSessionIds.includes('session-y'));
  });

  it('should detach agent from old project if registered to another project', () => {
    const projectRegistry = new ProjectRegistry();
    const agentRegistry = new AgentRegistry(tempDir, projectRegistry);

    // Create two projects
    const tempDirA = fs.mkdtempSync(path.join(os.tmpdir(), 'proj-a-'));
    const tempDirB = fs.mkdtempSync(path.join(os.tmpdir(), 'proj-b-'));

    try {
      const projA = projectRegistry.registerProject(tempDirA);
      const projB = projectRegistry.registerProject(tempDirB);

      const sessionId = 'migrating-session';

      // 1. Register to project A
      const agentInfo: AgentInfo = {
        name: 'migrating-agent',
        principalId: 'p-mig',
        sessionId,
        model: 'test',
        pid: 201,
        startedAt: Date.now(),
        sessionJsonlPath: '',
        status: 'online',
        controlMode: 'managed',
        messageCount: 0,
        totalInputTokens: 0,
        totalOutputTokens: 0,
        projectId: projA.projectId,
      };
      agentRegistry.registerAgent(agentInfo);

      // Verify attached to A
      assert.ok(projA.agentSessionIds.includes(sessionId));
      assert.ok(projA.managedAgentSessionIds.includes(sessionId));
      assert.ok(!projB.agentSessionIds.includes(sessionId));

      // 2. Re-register / migrate to project B
      agentInfo.projectId = projB.projectId;
      agentRegistry.registerAgent(agentInfo);

      // Verify attached to B and detached from A!
      assert.ok(!projA.agentSessionIds.includes(sessionId));
      assert.ok(!projA.managedAgentSessionIds.includes(sessionId));
      assert.ok(projB.agentSessionIds.includes(sessionId));
      assert.ok(projB.managedAgentSessionIds.includes(sessionId));

    } finally {
      fs.rmSync(tempDirA, { recursive: true, force: true });
      fs.rmSync(tempDirB, { recursive: true, force: true });
    }
  });

  it('should not mutate the original caller-provided agentInfo object on registerAgent', () => {
    const projectRegistry = new ProjectRegistry();
    const agentRegistry = new AgentRegistry(tempDir, projectRegistry);

    const project = projectRegistry.registerProject(tempDir);
    const sessionId = 'session-immutable-test';

    const originalAgentInfo: AgentInfo = {
      name: 'agent-immutable',
      principalId: 'p-imm',
      sessionId,
      model: 'test',
      pid: 301,
      startedAt: Date.now(),
      sessionJsonlPath: '',
      status: 'online',
      controlMode: 'managed',
      messageCount: 0,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      projectId: project.projectId,
    };

    agentRegistry.registerAgent(originalAgentInfo);

    // Assert that the original object is NOT mutated
    assert.strictEqual(originalAgentInfo.projectRoot, undefined);

    // Assert that the stored object in registry DOES have the correct properties
    const stored = agentRegistry.getAgent(sessionId)!;
    assert.strictEqual(stored.projectId, project.projectId);
    assert.strictEqual(stored.projectRoot, project.projectRoot);
  });

  it('should resolve project-scoped spawn config without mutating the caller object', () => {
    const projectRegistry = new ProjectRegistry();
    const project = projectRegistry.registerProject(tempDir);

    const spawnConfig: SpawnConfig = {
      cwd: '/some/arbitrary/path',
      principalId: 'p1',
      model: 'test',
      permissionMode: 'dangerous',
      appfsMountRoot: '/another/arbitrary/path',
      env: {},
      launchSpec: { kind: 'binary' as const, binaryPath: 'invalid-non-existent-binary' },
      projectId: project.projectId,
    };

    const resolved = resolveProjectScopedSpawnConfig(spawnConfig, projectRegistry);

    assert.notStrictEqual(resolved, spawnConfig);
    assert.strictEqual(resolved.cwd, project.projectRoot);
    assert.strictEqual(resolved.appfsMountRoot, project.mountRoot);
    assert.strictEqual(resolved.projectRoot, project.projectRoot);
    assert.strictEqual(spawnConfig.cwd, '/some/arbitrary/path');
    assert.strictEqual(spawnConfig.appfsMountRoot, '/another/arbitrary/path');
    assert.strictEqual(spawnConfig.projectRoot, undefined);
  });

  it('should associate registered managed and external agents correctly to projectSession arrays', () => {
    const projectRegistry = new ProjectRegistry();
    const agentRegistry = new AgentRegistry(tempDir, projectRegistry);

    const project = projectRegistry.registerProject(tempDir);

    // Register managed agent
    const managedAgent: AgentInfo = {
      name: 'managed-agent',
      principalId: 'p-man',
      sessionId: 'sess-man',
      model: 'test',
      pid: 101,
      startedAt: Date.now(),
      sessionJsonlPath: '',
      status: 'online',
      controlMode: 'managed',
      messageCount: 0,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      projectId: project.projectId,
    };
    agentRegistry.registerAgent(managedAgent);

    // Register external agent
    const externalAgent: AgentInfo = {
      name: 'external-agent',
      principalId: 'p-ext',
      sessionId: 'sess-ext',
      model: 'test',
      pid: 102,
      startedAt: Date.now(),
      sessionJsonlPath: '',
      status: 'online',
      controlMode: 'external',
      messageCount: 0,
      totalInputTokens: 0,
      totalOutputTokens: 0,
      projectId: project.projectId,
    };
    agentRegistry.registerAgent(externalAgent);

    // Verify projectSession arrays
    const updatedProject = projectRegistry.getProject(project.projectId)!;
    assert.ok(updatedProject.agentSessionIds.includes('sess-man'));
    assert.ok(updatedProject.agentSessionIds.includes('sess-ext'));
    
    assert.ok(updatedProject.managedAgentSessionIds.includes('sess-man'));
    assert.ok(!updatedProject.managedAgentSessionIds.includes('sess-ext'));
  });
});
