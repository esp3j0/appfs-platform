import { describe, it } from 'node:test';
import assert from 'node:assert';
import Fastify from 'fastify';
import { PrincipalLifecycleError, type PrincipalCreateRequest } from '../principal-lifecycle.js';
import { registerPrincipalsRoute } from './principals.js';

describe('Project scoped principals routes', () => {
  it('returns 404 for unknown project', async () => {
    const app = Fastify({ logger: false });
    registerPrincipalsRoute(app, fakeLifecycleService({
      listPrincipals: () => {
        throw new PrincipalLifecycleError(404, 'Project missing not found');
      },
    }));

    try {
      const res = await app.inject({
        method: 'GET',
        url: '/api/projects/missing/principals',
      });
      assert.strictEqual(res.statusCode, 404);
      assert.match(JSON.parse(res.payload).error, /Project missing not found/);
    } finally {
      await app.close();
    }
  });

  it('creates principal through lifecycle service', async () => {
    const calls: PrincipalCreateRequest[] = [];
    const app = Fastify({ logger: false });
    registerPrincipalsRoute(app, fakeLifecycleService({
      createPrincipal: (_projectId, body) => {
        calls.push(body);
        return Promise.resolve({
          status: 'created' as const,
          principal: { principal_id: body.principalId },
        });
      },
    }));

    try {
      const res = await app.inject({
        method: 'POST',
        url: '/api/projects/project-a/principals',
        payload: { principalId: 'coder' },
      });
      assert.strictEqual(res.statusCode, 201);
      assert.deepStrictEqual(JSON.parse(res.payload), {
        status: 'created',
        principal: { principal_id: 'coder' },
      });
      assert.deepStrictEqual(calls, [{ principalId: 'coder' }]);
    } finally {
      await app.close();
    }
  });

  it('starts principal through lifecycle service', async () => {
    const app = Fastify({ logger: false });
    registerPrincipalsRoute(app, fakeLifecycleService({
      startPrincipal: (projectId, principalId, body) => Promise.resolve({
        status: 'spawning' as const,
        spawnId: `${projectId}-${principalId}-${body?.model}`,
        principalId,
      }),
    }));

    try {
      const res = await app.inject({
        method: 'POST',
        url: '/api/projects/project-a/principals/coder/start',
        payload: { model: 'claude-opus-4-6' },
      });
      assert.strictEqual(res.statusCode, 200);
      assert.deepStrictEqual(JSON.parse(res.payload), {
        status: 'spawning',
        spawnId: 'project-a-coder-claude-opus-4-6',
        principalId: 'coder',
      });
    } finally {
      await app.close();
    }
  });

  it('maps lifecycle stop errors to HTTP status codes', async () => {
    const app = Fastify({ logger: false });
    registerPrincipalsRoute(app, fakeLifecycleService({
      stopPrincipal: () => {
        throw new PrincipalLifecycleError(404, 'No managed agent found for principal coder');
      },
    }));

    try {
      const res = await app.inject({
        method: 'POST',
        url: '/api/projects/project-a/principals/coder/stop',
      });
      assert.strictEqual(res.statusCode, 404);
      assert.match(JSON.parse(res.payload).error, /No managed agent/);
    } finally {
      await app.close();
    }
  });

  it('points legacy principal endpoint to project-scoped API', async () => {
    const app = Fastify({ logger: false });
    registerPrincipalsRoute(app, fakeLifecycleService());

    try {
      const res = await app.inject({
        method: 'GET',
        url: '/api/principals',
      });
      assert.strictEqual(res.statusCode, 400);
      assert.match(JSON.parse(res.payload).error, /\/api\/projects\/:projectId\/principals/);
    } finally {
      await app.close();
    }
  });
});

function fakeLifecycleService(overrides: Partial<FakeLifecycleService> = {}): FakeLifecycleService {
  return {
    listPrincipals: () => ({ version: 1, principals: [] }),
    createPrincipal: (_projectId, body) => Promise.resolve({
      status: 'created' as const,
      principal: { principal_id: body.principalId },
    }),
    deletePrincipal: (_projectId, principalId) => Promise.resolve({
      status: 'deleted' as const,
      principalId,
    }),
    startPrincipal: (_projectId, principalId) => Promise.resolve({
      status: 'spawning' as const,
      spawnId: 'spawn-1',
      principalId,
    }),
    stopPrincipal: (_projectId, principalId) => Promise.resolve({
      status: 'stopping' as const,
      principalId,
      sessionId: 'session-1',
    }),
    resumePrincipal: (_projectId, principalId) => Promise.resolve({
      status: 'spawning' as const,
      spawnId: 'spawn-1',
      principalId,
      sessionId: 'session-1',
    }),
    ...overrides,
  };
}

interface FakeLifecycleService {
  listPrincipals: (projectId: string) => unknown;
  createPrincipal: (projectId: string, body: PrincipalCreateRequest) => Promise<unknown>;
  deletePrincipal: (projectId: string, principalId: string) => Promise<unknown>;
  startPrincipal: (projectId: string, principalId: string, body?: { model?: string }) => Promise<unknown>;
  stopPrincipal: (projectId: string, principalId: string) => Promise<unknown>;
  resumePrincipal: (projectId: string, principalId: string, body?: { sessionId?: string }) => Promise<unknown>;
}
