import { describe, it, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import Fastify from 'fastify';
import cors from '@fastify/cors';
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

  it('should normalize compose mountpoint to project .appfs when opening a project', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      const composePath = path.join(tempDir, '.appfs-compose.yaml');
      fs.writeFileSync(
        composePath,
        'version: 1\nruntime:\n  db: .agentfs/demo.db\n  mountpoint: C:/mnt/appfs-external\n  backend: winfsp\n',
        'utf-8',
      );

      const res = await app.inject({
        method: 'POST',
        url: '/api/projects/open',
        payload: { projectRoot: tempDir },
      });

      assert.strictEqual(res.statusCode, 200);
      const data = JSON.parse(res.payload);
      assert.strictEqual(data.mountRoot, path.join(path.resolve(tempDir), '.appfs'));

      const updated = fs.readFileSync(composePath, 'utf-8');
      assert.match(updated, /mountpoint: \.\/\.appfs/);
      assert.doesNotMatch(updated, /C:\/mnt\/appfs-external/);
    } finally {
      await app.close();
    }
  });

  it('should support starting project with .appfs-compose.yml instead of .yaml', async () => {
    const app = Fastify({ logger: false });
    registerProjectsRoute(app, registry, controller);

    try {
      const project = registry.registerProject(tempDir);

      // Clean both compose files first
      const yamlPath = path.join(tempDir, '.appfs-compose.yaml');
      const ymlPath = path.join(tempDir, '.appfs-compose.yml');
      if (fs.existsSync(yamlPath)) fs.unlinkSync(yamlPath);
      if (fs.existsSync(ymlPath)) fs.unlinkSync(ymlPath);

      // Write ONLY .yml compose file
      fs.writeFileSync(ymlPath, 'version: "1"');

      // Ensure mountpoint is absent or empty
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
      assert.strictEqual(data.composeFilePath, ymlPath);
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

  describe('Compose Endpoints', () => {
    it('should return 404 when compose file is missing', async () => {
      const app = Fastify({ logger: false });
      registerProjectsRoute(app, registry, controller);

      try {
        const project = registry.registerProject(tempDir);
        const yamlPath = path.join(project.projectRoot, '.appfs-compose.yaml');
        const ymlPath = path.join(project.projectRoot, '.appfs-compose.yml');
        if (fs.existsSync(yamlPath)) fs.unlinkSync(yamlPath);
        if (fs.existsSync(ymlPath)) fs.unlinkSync(ymlPath);

        const res = await app.inject({
          method: 'GET',
          url: `/api/projects/${project.projectId}/compose`,
        });
        assert.strictEqual(res.statusCode, 404);
        const data = JSON.parse(res.payload);
        assert.ok(data.isMissing);
      } finally {
        await app.close();
      }
    });

    it('should read current compose content correctly', async () => {
      const app = Fastify({ logger: false });
      registerProjectsRoute(app, registry, controller);

      try {
        const project = registry.registerProject(tempDir);
        const yamlContent = 'version: 1\nruntime:\n  db: .agentfs/demo.db\n  mountpoint: ./mnt/appfs\n  backend: fuse\n';
        fs.writeFileSync(path.join(project.projectRoot, '.appfs-compose.yaml'), yamlContent, 'utf-8');

        const res = await app.inject({
          method: 'GET',
          url: `/api/projects/${project.projectId}/compose`,
        });
        assert.strictEqual(res.statusCode, 200);
        const data = JSON.parse(res.payload);
        assert.strictEqual(data.content, yamlContent);
      } finally {
        await app.close();
      }
    });

    it('should validate proposed compose content successfully', async () => {
      const app = Fastify({ logger: false });
      registerProjectsRoute(app, registry, controller);

      try {
        const project = registry.registerProject(tempDir);
        const validCompose = 'version: 1\nruntime:\n  db: .agentfs/demo.db\n  mountpoint: ./mnt/appfs\n  backend: fuse\n';

        const res = await app.inject({
          method: 'POST',
          url: `/api/projects/${project.projectId}/compose/validate`,
          payload: { content: validCompose },
        });
        assert.strictEqual(res.statusCode, 200);
        const data = JSON.parse(res.payload);
        assert.strictEqual(data.valid, true);
      } finally {
        await app.close();
      }
    });

    it('should reject invalid compose schema or syntax and return validation error', async () => {
      const app = Fastify({ logger: false });
      registerProjectsRoute(app, registry, controller);

      try {
        const project = registry.registerProject(tempDir);
        
        // 1. Invalid YAML syntax
        const invalidYaml = 'version: : : 1\n';
        const resSyntax = await app.inject({
          method: 'POST',
          url: `/api/projects/${project.projectId}/compose/validate`,
          payload: { content: invalidYaml },
        });
        assert.strictEqual(resSyntax.statusCode, 200);
        const dataSyntax = JSON.parse(resSyntax.payload);
        assert.strictEqual(dataSyntax.valid, false);
        assert.ok(dataSyntax.error);

        // 2. Syntactically valid YAML but invalid compose schema (unknown fields or missing required fields)
        const invalidSchema = 'version: 1\nruntime:\n  db: .agentfs/demo.db\n  unknown_field_xyz: true\n';
        const resSchema = await app.inject({
          method: 'POST',
          url: `/api/projects/${project.projectId}/compose/validate`,
          payload: { content: invalidSchema },
        });
        assert.strictEqual(resSchema.statusCode, 200);
        const dataSchema = JSON.parse(resSchema.payload);
        assert.strictEqual(dataSchema.valid, false);
        assert.ok(dataSchema.error);
      } finally {
        await app.close();
      }
    });

    it('should atomically save compose file and reject invalid saves without overwriting', async () => {
      const app = Fastify({ logger: false });
      registerProjectsRoute(app, registry, controller);

      try {
        const project = registry.registerProject(tempDir);
        const originalContent = 'version: 1\nruntime:\n  db: .agentfs/demo.db\n  mountpoint: ./mnt/appfs\n  backend: fuse\n';
        fs.writeFileSync(path.join(project.projectRoot, '.appfs-compose.yaml'), originalContent, 'utf-8');

        // 1. Try saving invalid compose content (should be rejected and original content preserved)
        const invalidContent = 'version: 1\nruntime:\n  db: .agentfs/demo.db\n  unknown_field: true\n';
        const resFail = await app.inject({
          method: 'PUT',
          url: `/api/projects/${project.projectId}/compose`,
          payload: { content: invalidContent },
        });
        assert.strictEqual(resFail.statusCode, 400);
        const dataFail = JSON.parse(resFail.payload);
        assert.match(dataFail.error, /Validation failed/);

        // Verify original file is unchanged
        const preservedContent = fs.readFileSync(path.join(project.projectRoot, '.appfs-compose.yaml'), 'utf-8');
        assert.strictEqual(preservedContent, originalContent);

        // 2. Save valid compose content (should succeed and update file)
        const newContent = 'version: 1\nruntime:\n  db: .agentfs/demo.db\n  mountpoint: ./mnt/appfs-new\n  backend: fuse\n';
        const resSuccess = await app.inject({
          method: 'PUT',
          url: `/api/projects/${project.projectId}/compose`,
          payload: { content: newContent },
        });
        assert.strictEqual(resSuccess.statusCode, 200);
        const dataSuccess = JSON.parse(resSuccess.payload);
        assert.strictEqual(dataSuccess.success, true);
        assert.match(dataSuccess.content, /mountpoint: \.\/\.appfs/);
        assert.strictEqual(dataSuccess.mountpointNormalized, true);

        // Verify original file is updated
        const updatedContent = fs.readFileSync(path.join(project.projectRoot, '.appfs-compose.yaml'), 'utf-8');
        assert.match(updatedContent, /mountpoint: \.\/\.appfs/);
        assert.doesNotMatch(updatedContent, /mnt\/appfs-new/);
      } finally {
        await app.close();
      }
    });
  });

  describe('CORS Restrictions in Packaged Mode', () => {
    it('should restrict CORS when ELECTRON_RUN_AS_NODE is 1', async () => {
      const originalEnv = process.env.ELECTRON_RUN_AS_NODE;
      process.env.ELECTRON_RUN_AS_NODE = '1';

      try {
        const app = Fastify({ logger: false });
        const corsOrigin = process.env.ELECTRON_RUN_AS_NODE === '1' ? false : ['http://localhost:5173', 'http://127.0.0.1:5173'];
        await app.register(cors, { origin: corsOrigin });

        app.get('/api/test', async () => ({ ok: true }));

        const res = await app.inject({
          method: 'GET',
          url: '/api/test',
          headers: {
            origin: 'http://malicious-origin.com'
          }
        });

        assert.strictEqual(res.headers['access-control-allow-origin'], undefined);
      } finally {
        process.env.ELECTRON_RUN_AS_NODE = originalEnv;
      }
    });

    it('should allow CORS ONLY for Vite dev loopbacks in development mode', async () => {
      const originalEnv = process.env.ELECTRON_RUN_AS_NODE;
      delete process.env.ELECTRON_RUN_AS_NODE;

      try {
        const app = Fastify({ logger: false });
        const corsOrigin = process.env.ELECTRON_RUN_AS_NODE === '1' ? false : ['http://localhost:5173', 'http://127.0.0.1:5173'];
        await app.register(cors, { origin: corsOrigin });
        app.get('/api/test', async () => ({ ok: true }));

        // Allow localhost:5173
        const res1 = await app.inject({
          method: 'GET',
          url: '/api/test',
          headers: {
            origin: 'http://localhost:5173'
          }
        });
        assert.strictEqual(res1.headers['access-control-allow-origin'], 'http://localhost:5173');

        // Allow 127.0.0.1:5173
        const res2 = await app.inject({
          method: 'GET',
          url: '/api/test',
          headers: {
            origin: 'http://127.0.0.1:5173'
          }
        });
        assert.strictEqual(res2.headers['access-control-allow-origin'], 'http://127.0.0.1:5173');

        // Reject malicious origin in dev mode
        const res3 = await app.inject({
          method: 'GET',
          url: '/api/test',
          headers: {
            origin: 'http://malicious-origin.com'
          }
        });
        assert.strictEqual(res3.headers['access-control-allow-origin'], undefined);
      } finally {
        process.env.ELECTRON_RUN_AS_NODE = originalEnv;
      }
    });
  });
});
