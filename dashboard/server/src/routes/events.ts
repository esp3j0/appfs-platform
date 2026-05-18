import type { FastifyInstance, FastifyReply } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { MessageRecord, TimelineEntry, ContentBlock } from '../types.js';
import { FileWatcher } from '../file-watcher.js';

const SSE_CLIENTS: Set<FastifyReply> = new Set();

function sseSend(reply: FastifyReply, event: string, data: unknown): void {
  reply.raw.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
}

export function registerEventsRoute(app: FastifyInstance, registry: AgentRegistry): void {
  const watcher = new FileWatcher(registry);

  watcher.start((agentName: string, newRecords: MessageRecord[]) => {
    for (const rec of newRecords) {
      const msg = rec.message;
      const entry: TimelineEntry = {
        id: `${agentName}:${msg.uuid}`,
        agentName,
        timestamp: msg.timestamp_ms ?? Date.now(),
        source: 'session',
        role: msg.role,
        content: extractTextContent(msg.blocks),
        raw: msg,
        usage: msg.usage,
      };
      for (const client of SSE_CLIENTS) {
        sseSend(client, 'message', entry);
      }
    }
  });

  app.get('/api/events', async (request, reply) => {
    reply.raw.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
    });
    reply.raw.write('\n');
    SSE_CLIENTS.add(reply);

    request.raw.on('close', () => {
      SSE_CLIENTS.delete(reply);
    });

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
