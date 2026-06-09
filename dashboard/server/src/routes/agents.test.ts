import { describe, it } from 'node:test';
import assert from 'node:assert';
import Fastify from 'fastify';
import { registerAgentsRoute } from './agents.js';
import type { AgentInfo } from '../types.js';

describe('Agents route', () => {
  it('overlays current managed process state onto discovered registry agents', async () => {
    const app = Fastify({ logger: false });
    const registryAgent: AgentInfo = {
      name: 'default',
      principalId: 'default',
      sessionId: 'session-default',
      model: 'claude-opus-4-6',
      pid: 0,
      startedAt: 1000,
      sessionJsonlPath: 'session-default.jsonl',
      status: 'offline',
      controlMode: 'external',
      messageCount: 1,
      totalInputTokens: 10,
      totalOutputTokens: 2,
      projectId: 'project-a',
    };

    registerAgentsRoute(
      app,
      fakeRegistry([registryAgent]),
      {
        getManagedAgents: () => [{
          pid: 123,
          sessionId: 'session-default',
          status: 'idle',
          principalId: 'default',
          projectId: 'project-a',
          model: 'claude-opus-4-6',
          permissionMode: 'dangerous',
        }],
      } as any,
    );

    try {
      const res = await app.inject({ method: 'GET', url: '/api/agents' });
      assert.strictEqual(res.statusCode, 200);
      const agents = JSON.parse(res.payload);
      assert.strictEqual(agents[0].status, 'online');
      assert.strictEqual(agents[0].controlMode, 'managed');
      assert.strictEqual(agents[0].pid, 123);
      assert.strictEqual(agents[0].messageCount, 1);
    } finally {
      await app.close();
    }
  });

  it('filters archived agents by default and exposes archived query modes', async () => {
    const app = Fastify({ logger: false });
    const active = agentInfo({ sessionId: 'session-active', principalId: 'default' });
    const archived = agentInfo({
      sessionId: 'session-archived',
      principalId: 'coder',
      archived: true,
      archivedAt: 1234,
      archivedReason: 'principal_deleted',
    });

    registerAgentsRoute(app, fakeRegistry([active, archived]));

    try {
      const defaultRes = await app.inject({ method: 'GET', url: '/api/agents' });
      assert.strictEqual(defaultRes.statusCode, 200);
      assert.deepStrictEqual(
        JSON.parse(defaultRes.payload).map((agent: AgentInfo) => agent.sessionId),
        ['session-active'],
      );

      const archivedRes = await app.inject({ method: 'GET', url: '/api/agents?archived=only' });
      assert.strictEqual(archivedRes.statusCode, 200);
      assert.deepStrictEqual(
        JSON.parse(archivedRes.payload).map((agent: AgentInfo) => agent.sessionId),
        ['session-archived'],
      );

      const includeRes = await app.inject({ method: 'GET', url: '/api/agents?archived=include' });
      assert.strictEqual(includeRes.statusCode, 200);
      assert.deepStrictEqual(
        JSON.parse(includeRes.payload).map((agent: AgentInfo) => agent.sessionId),
        ['session-active', 'session-archived'],
      );
    } finally {
      await app.close();
    }
  });
});

function fakeRegistry(agents: AgentInfo[]) {
  return {
    getAgents: () => agents,
    getActiveAgents: () => agents.filter(agent => !agent.archived),
    getArchivedAgents: () => agents.filter(agent => agent.archived),
  } as any;
}

function agentInfo(overrides: Partial<AgentInfo> & { sessionId: string; principalId: string }): AgentInfo {
  const { principalId, sessionId, ...rest } = overrides;
  return {
    name: principalId,
    principalId,
    sessionId,
    model: 'claude-opus-4-6',
    pid: 0,
    startedAt: 1000,
    sessionJsonlPath: `${sessionId}.jsonl`,
    status: 'offline',
    controlMode: 'external',
    messageCount: 0,
    totalInputTokens: 0,
    totalOutputTokens: 0,
    ...rest,
  };
}
