import { describe, it } from 'node:test';
import assert from 'node:assert';
import Fastify from 'fastify';
import { registerProcessRoute } from './process.js';
import { AgentNoActiveTurnError } from '../process-manager.js';
describe('Process Route Spawn Config', () => {
    it('allows project-scoped spawn requests to defer cwd and mount resolution', async () => {
        const app = Fastify({ logger: false });
        let receivedConfig;
        const processManager = {
            getDefaultSpawnConfig: () => ({
                cwd: '',
                principalId: 'default',
                model: 'claude-opus-4-6',
                permissionMode: 'dangerous',
                appfsMountRoot: '',
                appfsIdleWake: true,
                env: {},
                launchSpec: { kind: 'binary', binaryPath: 'claw.exe' },
            }),
            spawn: (config) => {
                receivedConfig = config;
                return { spawnId: 'spawn-test' };
            },
            sendPrompt: async () => ({ requestId: 'req-test', status: 'accepted' }),
            promoteQueuedInput: async () => ({ requestId: 'req-test', status: 'guidance' }),
            cancelTurn: async () => ({ requestId: 'req-test', status: 'cancelling' }),
            stop: () => true,
            getStatus: () => ({ status: 'idle', currentRequestId: null }),
            getManagedSessionIds: () => [],
        };
        registerProcessRoute(app, processManager);
        try {
            const payload = {
                cwd: '',
                principalId: 'default',
                model: 'claude-opus-4-6',
                permissionMode: 'dangerous',
                appfsMountRoot: '',
                appfsIdleWake: true,
                env: {},
                launchSpec: { kind: 'binary', binaryPath: 'claw.exe' },
                projectId: 'project-123',
            };
            const res = await app.inject({
                method: 'POST',
                url: '/api/process/spawn',
                payload,
            });
            assert.strictEqual(res.statusCode, 201);
            assert.strictEqual(JSON.parse(res.payload).spawnId, 'spawn-test');
            assert.strictEqual(receivedConfig?.projectId, 'project-123');
        }
        finally {
            await app.close();
        }
    });
    it('still rejects non-project spawns without cwd and mount root', async () => {
        const app = Fastify({ logger: false });
        const processManager = {
            getDefaultSpawnConfig: () => ({}),
            spawn: () => ({ spawnId: 'should-not-spawn' }),
            sendPrompt: async () => ({ requestId: 'req-test', status: 'accepted' }),
            promoteQueuedInput: async () => ({ requestId: 'req-test', status: 'guidance' }),
            cancelTurn: async () => ({ requestId: 'req-test', status: 'cancelling' }),
            stop: () => true,
            getStatus: () => ({ status: 'idle', currentRequestId: null }),
            getManagedSessionIds: () => [],
        };
        registerProcessRoute(app, processManager);
        try {
            const res = await app.inject({
                method: 'POST',
                url: '/api/process/spawn',
                payload: {
                    cwd: '',
                    principalId: 'default',
                    model: 'claude-opus-4-6',
                    permissionMode: 'dangerous',
                    appfsMountRoot: '',
                    env: {},
                    launchSpec: { kind: 'binary', binaryPath: 'claw.exe' },
                },
            });
            assert.strictEqual(res.statusCode, 400);
            assert.match(JSON.parse(res.payload).error, /cwd/);
            assert.match(JSON.parse(res.payload).error, /appfsMountRoot/);
        }
        finally {
            await app.close();
        }
    });
    it('stops a managed agent by session id', async () => {
        const app = Fastify({ logger: false });
        let stoppedSessionId;
        const processManager = {
            getDefaultSpawnConfig: () => ({}),
            spawn: () => ({ spawnId: 'unused' }),
            sendPrompt: async () => ({ requestId: 'req-test', status: 'accepted' }),
            promoteQueuedInput: async () => ({ requestId: 'req-test', status: 'guidance' }),
            cancelTurn: async () => ({ requestId: 'req-test', status: 'cancelling' }),
            stop: (sessionId) => {
                stoppedSessionId = sessionId;
                return true;
            },
            getStatus: () => ({ status: 'idle', currentRequestId: null }),
            getManagedSessionIds: () => [],
        };
        registerProcessRoute(app, processManager);
        try {
            const res = await app.inject({
                method: 'POST',
                url: '/api/agents/session-123/stop',
            });
            assert.strictEqual(res.statusCode, 200);
            assert.strictEqual(stoppedSessionId, 'session-123');
            assert.deepStrictEqual(JSON.parse(res.payload), {
                status: 'stopping',
                sessionId: 'session-123',
            });
        }
        finally {
            await app.close();
        }
    });
    it('cancels the active turn without stopping the agent process', async () => {
        const app = Fastify({ logger: false });
        let cancelledSessionId;
        let cancelledRequestId;
        let stopped = false;
        const processManager = {
            getDefaultSpawnConfig: () => ({}),
            spawn: () => ({ spawnId: 'unused' }),
            sendPrompt: async () => ({ requestId: 'req-test', status: 'accepted' }),
            promoteQueuedInput: async () => ({ requestId: 'req-test', status: 'guidance' }),
            cancelTurn: async (sessionId, requestId) => {
                cancelledSessionId = sessionId;
                cancelledRequestId = requestId;
                return { requestId: requestId ?? 'req-active', status: 'cancelling' };
            },
            stop: () => {
                stopped = true;
                return true;
            },
            getStatus: () => ({ status: 'busy', currentRequestId: 'req-active' }),
            getManagedSessionIds: () => [],
        };
        registerProcessRoute(app, processManager);
        try {
            const res = await app.inject({
                method: 'POST',
                url: '/api/agents/session-123/cancel-turn',
                payload: { request_id: 'req-456' },
            });
            assert.strictEqual(res.statusCode, 200);
            assert.strictEqual(cancelledSessionId, 'session-123');
            assert.strictEqual(cancelledRequestId, 'req-456');
            assert.strictEqual(stopped, false);
            assert.deepStrictEqual(JSON.parse(res.payload), {
                status: 'cancelling',
                request_id: 'req-456',
            });
        }
        finally {
            await app.close();
        }
    });
    it('returns conflict when there is no active turn to cancel', async () => {
        const app = Fastify({ logger: false });
        const processManager = {
            getDefaultSpawnConfig: () => ({}),
            spawn: () => ({ spawnId: 'unused' }),
            sendPrompt: async () => ({ requestId: 'req-test', status: 'accepted' }),
            promoteQueuedInput: async () => ({ requestId: 'req-test', status: 'guidance' }),
            cancelTurn: async (sessionId) => {
                throw new AgentNoActiveTurnError(sessionId);
            },
            stop: () => true,
            getStatus: () => ({ status: 'idle', currentRequestId: null }),
            getManagedSessionIds: () => [],
        };
        registerProcessRoute(app, processManager);
        try {
            const res = await app.inject({
                method: 'POST',
                url: '/api/agents/session-123/cancel-turn',
            });
            assert.strictEqual(res.statusCode, 409);
            assert.match(JSON.parse(res.payload).error, /active turn/);
        }
        finally {
            await app.close();
        }
    });
});
