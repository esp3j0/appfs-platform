import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';

export function registerMessagesRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/agents/:name/messages', async (request) => {
    const { name } = request.params as { name: string };
    const messages = registry.getMessages(decodeURIComponent(name));
    return messages;
  });
}
