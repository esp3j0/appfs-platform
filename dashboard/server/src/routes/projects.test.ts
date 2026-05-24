import { describe, it, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import Fastify from 'fastify';
import { ProjectRegistry } from '../project-registry.js';
import { registerProjectsRoute } from './projects.js';
import type { ProjectRuntimeController } from './projects.js';
import type { ProjectRecord } from '../project-registry.js';

class FakeProjectRuntimeController implements ProjectRuntimeController {
  public stoppedSessionIds: string[] = [];

  constructor(private registry: ProjectRegistry) {}

  async start(projectId: string): Promise<ProjectRecord> {
    const project = this.registry.getProject(projectId);
    if (!project) {
      throw new Error('Project not found');
    }
    project.status = 'running';
    return project;
  }

  async stop(projectId: string): Promise<ProjectRecord> {
    const project = this.registry.getProject(projectId);
    if (!project) {
      throw new Error('Project not found');
    }
    // Record stopped sessions
    this.stoppedSessionIds.push(...project.managedAgentSessionIds);
    project.status = 'stopped';
    return project;
  }

  status(projectId: string): ProjectRecord | undefined {
    return this.registry.getProject(projectId);
  }
}

describe('Projects Route Lifecycle (P1)', () => {
  let tempDir: string;
  let registry: ProjectRegistry;
  let controller: FakeProjectRuntimeController;

  beforeEach(() => {
    tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'appfs-route-test-'));
    registry = new ProjectRegistry();
    controller = new FakeProjectRuntimeController(registry);
  });

  afterEach(() => {
    fs.rmSync(tempDir, { recursive: true, force: true });
  });

  it('should return 404 for unknown projects on status, start, and stop', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      let res = await app.inject({
        method: 'GET',
        url: '/api/projects/unknown-id/status',
      });
      assert.strictEqual(res.statusCode, 404);

      res = await app.inject({
        method: 'POST',
        url: '/api/projects/unknown-id/start',
      });
      assert.strictEqual(res.statusCode, 404);

      res = await app.inject({
        method: 'POST',
        url: '/api/projects/unknown-id/stop',
      });
      assert.strictEqual(res.statusCode, 404);
    } finally {
      await app.close();
    }
  });

  it('should return 400 on start when compose file is missing', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      const project = registry.registerProject(tempDir);
      
      // Make sure compose file doesn't exist
      const composePath = path.join(tempDir, '.appfs-compose.yaml');
      if (fs.existsSync(composePath)) {
        fs.unlinkSync(composePath);
      }

      const res = await app.inject({
        method: 'POST',
        url: `/api/projects/${project.projectId}/start`,
      });
      assert.strictEqual(res.statusCode, 400);
      const data = JSON.parse(res.payload);
      assert.match(data.error, /Missing compose file/);
    } finally {
      await app.close();
    }
  });

  it('should return 400/409 on start when mountpoint conflict exists', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      const project = registry.registerProject(tempDir);

      // Create compose file to satisfy previous step check
      fs.writeFileSync(path.join(tempDir, '.appfs-compose.yaml'), 'version: "1"');

      // Create a non-empty .appfs directory
      const mountPath = path.join(tempDir, '.appfs');
      if (!fs.existsSync(mountPath)) {
        fs.mkdirSync(mountPath);
      }
      fs.writeFileSync(path.join(mountPath, 'conflict.txt'), 'conflict content');

      let res = await app.inject({
        method: 'POST',
        url: `/api/projects/${project.projectId}/start`,
      });
      assert.ok(res.statusCode === 400 || res.statusCode === 409);
      let data = JSON.parse(res.payload);
      assert.match(data.error, /Conflict detected/);

      // Make it a file instead
      fs.rmSync(mountPath, { recursive: true, force: true });
      fs.writeFileSync(mountPath, 'file content');

      res = await app.inject({
        method: 'POST',
        url: `/api/projects/${project.projectId}/start`,
      });
      assert.ok(res.statusCode === 400 || res.statusCode === 409);
      data = JSON.parse(res.payload);
      assert.match(data.error, /Conflict detected/);
    } finally {
      await app.close();
    }
  });

  it('should start project status transitioning to running when conditions are met', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      const project = registry.registerProject(tempDir);

      // Write compose file
      fs.writeFileSync(path.join(tempDir, '.appfs-compose.yaml'), 'version: "1"');

      // Ensure mountpoint is absent or empty (absent is fine)
      const mountPath = path.join(tempDir, '.appfs');
      if (fs.existsSync(mountPath)) {
        fs.rmSync(mountPath, { recursive: true, force: true });
      }

      const res = await app.inject({
        method: 'POST',
        url: `/api/projects/${project.projectId}/start`,
      });
      assert.strictEqual(res.statusCode, 200);
      const data = JSON.parse(res.payload);
      assert.strictEqual(data.status, 'running');
    } finally {
      await app.close();
    }
  });

  it('should stop project and only stop managed agents', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      const project = registry.registerProject(tempDir);

      // Set to running
      project.status = 'running';

      // Setup agents
      project.agentSessionIds = ['managed-session', 'external-session'];
      project.managedAgentSessionIds = ['managed-session'];

      const res = await app.inject({
        method: 'POST',
        url: `/api/projects/${project.projectId}/stop`,
      });
      assert.strictEqual(res.statusCode, 200);
      const data = JSON.parse(res.payload);
      assert.strictEqual(data.status, 'stopped');

      // Assert only managed session was stopped
      assert.deepStrictEqual(controller.stoppedSessionIds, ['managed-session']);
    } finally {
      await app.close();
    }
  });

  it('should support checking status and returning all project fields', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      const project = registry.registerProject(tempDir);

      const res = await app.inject({
        method: 'GET',
        url: `/api/projects/${project.projectId}/status`,
      });
      assert.strictEqual(res.statusCode, 200);
      const data = JSON.parse(res.payload);

      assert.strictEqual(data.projectId, project.projectId);
      assert.strictEqual(data.projectRoot, project.projectRoot);
      assert.strictEqual(data.composeFilePath, project.composeFilePath);
      assert.strictEqual(data.mountRoot, project.mountRoot);
      assert.strictEqual(data.status, project.status);
      assert.deepStrictEqual(data.agentSessionIds, project.agentSessionIds);
      assert.deepStrictEqual(data.managedAgentSessionIds, project.managedAgentSessionIds);
    } finally {
      await app.close();
    }
  });

  it('should transition project status to error and return 500 when starting fails', async () => {
    const errorController: ProjectRuntimeController = {
      start: async () => { throw new Error('Simulated spawn error'); },
      stop: async () => { throw new Error('Not implemented'); },
      status: () => undefined,
    };
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, errorController);

    try {
      const project = registry.registerProject(tempDir);
      // Write compose file to pass initial check
      fs.writeFileSync(path.join(tempDir, '.appfs-compose.yaml'), 'version: "1"');

      const res = await app.inject({
        method: 'POST',
        url: `/api/projects/${project.projectId}/start`,
      });
      assert.strictEqual(res.statusCode, 500);
      const data = JSON.parse(res.payload);
      assert.match(data.error, /Simulated spawn error/);

      // Verify state is transitioned to error
      const updatedProject = registry.getProject(project.projectId);
      assert.strictEqual(updatedProject?.status, 'error');
    } finally {
      await app.close();
    }
  });
});
