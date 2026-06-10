import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import { normalizeChatThread } from '../chat-normalizer.js';

export function registerMessagesRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/agents/:name/messages', async (request) => {
    const { name } = request.params as { name: string };
    const messages = registry.getMessages(decodeURIComponent(name));
    return messages;
  });

  app.get('/api/agents/:sessionId/chat', async (request) => {
    const { sessionId } = request.params as { sessionId: string };
    const decodedSessionId = decodeURIComponent(sessionId);
    return normalizeChatThread(
      decodedSessionId,
      registry.getMessages(decodedSessionId),
      registry.getTurnErrors(decodedSessionId),
    );
  });
}
