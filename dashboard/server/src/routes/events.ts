import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { MessageRecord, TimelineEntry, ContentBlock } from '../types.js';
import { FileWatcher } from '../file-watcher.js';
import { EventBus } from '../event-bus.js';

export function registerEventsRoute(app: FastifyInstance, registry: AgentRegistry): void {
  const eventBus = EventBus.getInstance();
  const watcher = new FileWatcher(registry);

  // Register watcher on the registry so other components can dynamically add paths to it
  registry.setFileWatcher(watcher);

  watcher.start((sessionId: string, newRecords: MessageRecord[]) => {
    const agent = registry.getAgent(sessionId);
    const agentName = agent?.name ?? sessionId;

    for (const rec of newRecords) {
      const msg = rec.message;
      const entry: TimelineEntry = {
        id: `${sessionId}:${msg.uuid}`,
        sessionId,
        agentName,
        timestamp: msg.timestamp_ms ?? Date.now(),
        source: 'session',
        role: msg.role,
        content: extractTextContent(msg.blocks),
        raw: msg,
        usage: msg.usage,
      };

      // Broadcast to SSE clients via the unified EventBus
      eventBus.broadcast('message', entry);
    }
  });

  app.get('/api/events', async (request, reply) => {
    eventBus.registerClient(request, reply);
    // Keep the response open
    await new Promise(() => {});
  });
}

function extractTextContent(blocks: ContentBlock[]): string {
  return blocks
    .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
    .map(b => b.text)
    .join('\n');
}
