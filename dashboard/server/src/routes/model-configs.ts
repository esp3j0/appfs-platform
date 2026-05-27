import type { FastifyInstance } from 'fastify';
import {
  ModelConfigStore,
  ModelConfigValidationError,
  type DashboardModelConfig,
} from '../model-config-store.js';

export function registerModelConfigsRoute(
  app: FastifyInstance,
  modelConfigStore: ModelConfigStore,
): void {
  app.get('/api/model-configs', async (_request, reply) => {
    try {
      return reply.status(200).send({
        config: modelConfigStore.load(),
        path: modelConfigStore.getConfigPath(),
      });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      return reply.status(500).send({ error: message });
    }
  });

  app.put<{
    Body: DashboardModelConfig;
  }>('/api/model-configs', async (request, reply) => {
    try {
      const config = modelConfigStore.save(request.body);
      return reply.status(200).send({
        config,
        path: modelConfigStore.getConfigPath(),
      });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      const status = err instanceof ModelConfigValidationError ? 400 : 500;
      return reply.status(status).send({ error: message });
    }
  });
}

