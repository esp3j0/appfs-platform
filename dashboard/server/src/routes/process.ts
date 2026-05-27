import type { FastifyInstance } from 'fastify';
import {
  AgentNoActiveTurnError,
  AgentProcessManager,
  SpawnConfigValidationError,
  type PromptDelivery,
  type SpawnConfig,
} from '../process-manager.js';

export function registerProcessRoute(app: FastifyInstance, processManager: AgentProcessManager): void {
  app.get('/api/process/default-spawn-config', async (_request, reply) => {
    return reply.status(200).send(processManager.getDefaultSpawnConfig());
  });

  // ── Spawn a new headless agent ──
  app.post<{
    Body: SpawnConfig;
  }>('/api/process/spawn', async (request, reply) => {
    try {
      const config = request.body || ({} as SpawnConfig);

      // Validate required fields
      const missing: string[] = [];
      if (!config.principalId?.trim()) missing.push('principalId');
      if (!config.model?.trim()) missing.push('model');
      if (!config.launchSpec) missing.push('launchSpec');
      if (!config.projectId) {
        if (!config.cwd?.trim()) missing.push('cwd');
        if (!config.appfsMountRoot?.trim()) missing.push('appfsMountRoot');
      }

      if (missing.length > 0) {
        return reply.status(400).send({
          error: `Missing required fields: ${missing.join(', ')}`,
        });
      }

      const { spawnId } = processManager.spawn(config);
      return reply.status(201).send({ spawnId, status: 'spawning' });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      console.error('[ProcessRoute] Spawn error:', message);
      if (err instanceof SpawnConfigValidationError) {
        return reply.status(400).send({ error: message });
      }
      return reply.status(500).send({ error: message });
    }
  });

  // ── Send a prompt to a managed agent ──
  app.post<{
    Params: { sessionId: string };
    Body: { prompt: string; delivery?: PromptDelivery };
  }>('/api/agents/:sessionId/prompt', async (request, reply) => {
    try {
      const { sessionId } = request.params;
      const { prompt, delivery } = request.body;

      if (!prompt || typeof prompt !== 'string' || prompt.trim().length === 0) {
        return reply.status(400).send({ error: 'prompt is required and must be a non-empty string' });
      }
      if (delivery && !['prompt', 'queue', 'guidance'].includes(delivery)) {
        return reply.status(400).send({ error: 'delivery must be one of: prompt, queue, guidance' });
      }

      const { requestId, status } = await processManager.sendPrompt(sessionId, prompt.trim(), delivery);
      return reply.status(200).send({ request_id: requestId, status });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      return reply.status(404).send({ error: message });
    }
  });

  // ── Promote a queued chat input into running guidance ──
  app.post<{
    Params: { sessionId: string; requestId: string };
  }>('/api/agents/:sessionId/inputs/:requestId/guidance', async (request, reply) => {
    try {
      const { sessionId, requestId } = request.params;
      const result = await processManager.promoteQueuedInput(sessionId, requestId);
      return reply.status(200).send({ request_id: result.requestId, status: result.status });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      return reply.status(404).send({ error: message });
    }
  });

  // ── Cancel the currently running turn without killing the managed agent ──
  app.post<{
    Params: { sessionId: string };
    Body: { request_id?: string; requestId?: string };
  }>('/api/agents/:sessionId/cancel-turn', async (request, reply) => {
    try {
      const { sessionId } = request.params;
      const requestId = request.body?.request_id ?? request.body?.requestId;
      const result = await processManager.cancelTurn(sessionId, requestId);
      return reply.status(200).send({ request_id: result.requestId, status: result.status });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      const status = err instanceof AgentNoActiveTurnError || message.includes('still starting') ? 409 : 404;
      return reply.status(status).send({ error: message });
    }
  });

  // ── Stop a managed agent ──
  app.post<{
    Params: { sessionId: string };
  }>('/api/agents/:sessionId/stop', async (request, reply) => {
    const { sessionId } = request.params;
    const stopped = processManager.stop(sessionId);

    if (!stopped) {
      return reply.status(404).send({ error: `No managed agent found for sessionId: ${sessionId}` });
    }

    return reply.status(200).send({ status: 'stopping', sessionId });
  });

  // ── Get agent process status ──
  app.get<{
    Params: { sessionId: string };
  }>('/api/agents/:sessionId/status', async (request, reply) => {
    const { sessionId } = request.params;
    const status = processManager.getStatus(sessionId);

    if (!status) {
      return reply.status(404).send({ error: `No managed agent found for sessionId: ${sessionId}` });
    }

    return reply.status(200).send(status);
  });

  // ── List all managed agent sessions ──
  app.get('/api/process/managed', async (_request, reply) => {
    const sessionIds = processManager.getManagedSessionIds();
    return reply.status(200).send({ managed: sessionIds });
  });
}
