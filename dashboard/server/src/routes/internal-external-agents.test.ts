import { describe, it } from 'node:test';
import assert from 'node:assert';
import Fastify from 'fastify';
import {
  PrincipalLifecycleError,
  type PrincipalCreateRequest,
  type PrincipalStartRequest,
} from '../principal-lifecycle.js';
import { registerInternalExternalAgentsRoute } from './internal-external-agents.js';

describe('Internal external agent route', () => {
  it('rejects requests without the dashboard control token', async () => {
    const app = Fastify({ logger: false });
    registerInternalExternalAgentsRoute(app, fakeLifecycleService(), 'secret-token');

    try {
      const res = await app.inject({
        method: 'POST',
        url: '/api/internal/projects/project-a/external-agents',
        payload: { principalId: 'coder' },
      });
      assert.strictEqual(res.statusCode, 403);
    } finally {
      await app.close();
    }
  });

  it('ensures principal and starts it through lifecycle service', async () => {
    const calls: Array<{ method: string; projectId: string; body?: unknown; principalId?: string }> = [];
    const app = Fastify({ logger: false });
    registerInternalExternalAgentsRoute(app, fakeLifecycleService({
      ensurePrincipalReady: (projectId, body) => {
        calls.push({ method: 'ensure', projectId, body });
        return Promise.resolve({ status: 'created' });
      },
      startPrincipalAndWait: (projectId, principalId, body) => {
        calls.push({ method: 'start', projectId, principalId, body });
        return Promise.resolve({
          status: 'started',
          principalId,
          spawnId: 'spawn-1',
          sessionId: 'session-1',
          model: body?.model,
        });
      },
    }), 'secret-token');

    try {
      const res = await app.inject({
        method: 'POST',
        url: '/api/internal/projects/project-a/external-agents',
        headers: { 'x-appfs-agent-control-token': 'secret-token' },
        payload: {
          principalId: 'coder',
          displayName: 'Coder',
          description: 'External teammate',
          model: 'claude-opus-test',
          permissionMode: 'workspace-write',
          teamName: 'alpha team',
          taskListId: 'alpha',
        },
      });

      assert.strictEqual(res.statusCode, 200);
      assert.deepStrictEqual(JSON.parse(res.payload), {
        status: 'started',
        principalId: 'coder',
        spawnId: 'spawn-1',
        sessionId: 'session-1',
        model: 'claude-opus-test',
      });
      assert.deepStrictEqual(calls, [
        {
          method: 'ensure',
          projectId: 'project-a',
          body: {
            principalId: 'coder',
            displayName: 'Coder',
            description: 'External teammate',
            kind: 'agent',
          },
        },
        {
          method: 'start',
          projectId: 'project-a',
          principalId: 'coder',
          body: {
            model: 'claude-opus-test',
            modelProviderId: undefined,
            modelId: undefined,
            contextWindowTokens: undefined,
            maxOutputTokens: undefined,
            permissionMode: 'workspace-write',
            teamName: 'alpha team',
            taskListId: 'alpha',
          },
        },
      ]);
    } finally {
      await app.close();
    }
  });

  it('maps lifecycle machine codes to response codes', async () => {
    const app = Fastify({ logger: false });
    registerInternalExternalAgentsRoute(app, fakeLifecycleService({
      startPrincipalAndWait: () => {
        throw new PrincipalLifecycleError(
          409,
          'Principal coder already has a managed agent that is still starting',
          'AGENT_STARTING',
        );
      },
    }), 'secret-token');

    try {
      const res = await app.inject({
        method: 'POST',
        url: '/api/internal/projects/project-a/external-agents',
        headers: { 'x-appfs-agent-control-token': 'secret-token' },
        payload: { principalId: 'coder' },
      });
      assert.strictEqual(res.statusCode, 409);
      assert.deepStrictEqual(JSON.parse(res.payload), {
        error: 'Principal coder already has a managed agent that is still starting',
        code: 'AGENT_STARTING',
      });
    } finally {
      await app.close();
    }
  });
});

function fakeLifecycleService(
  overrides: Partial<FakeLifecycleService> = {},
): FakeLifecycleService {
  return {
    ensurePrincipalReady: (_projectId, body) => Promise.resolve({
      status: 'created' as const,
      principal: { principal_id: body.principalId },
    }),
    startPrincipalAndWait: (_projectId, principalId) => Promise.resolve({
      status: 'started' as const,
      principalId,
      spawnId: 'spawn-1',
      sessionId: 'session-1',
    }),
    ...overrides,
  };
}

interface FakeLifecycleService {
  ensurePrincipalReady: (projectId: string, body: PrincipalCreateRequest) => Promise<unknown>;
  startPrincipalAndWait: (
    projectId: string,
    principalId: string,
    body?: PrincipalStartRequest,
  ) => Promise<unknown>;
}
