import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';

export function registerAgentsRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/agents', async () => {
    return registry.getAgents();
  });
}
