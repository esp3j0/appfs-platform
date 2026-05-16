import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { TimelineEntry, ContentBlock, CrossAgentInteraction } from '../types.js';

export function registerTimelineRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/timeline', async (request) => {
    const query = request.query as { agents?: string };
    const agentNames = (query.agents ?? '').split(',').filter(Boolean).map(decodeURIComponent);

    if (agentNames.length === 0) {
      return { entries: [], interactions: [] };
    }

    const entries: TimelineEntry[] = [];
    const interactions: CrossAgentInteraction[] = [];

    for (const name of agentNames) {
      const msgs = registry.getMessages(name);
      for (const rec of msgs) {
        const msg = rec.message;
        const content = extractTextContent(msg.blocks);
        const timestamp = rec.timestamp_ms ?? 0;
        const entry: TimelineEntry = {
          agentName: name,
          timestamp,
          source: 'session',
          role: msg.role,
          content,
          raw: msg,
          usage: msg.usage,
        };
        entries.push(entry);

        // Detect cross-agent interactions from user messages
        if (msg.role === 'user') {
          const receivedMatch = content.match(/\[appfs_event\]\s+type=message\.received\s+from=(\S+)/);
          if (receivedMatch) {
            interactions.push({
              fromAgent: receivedMatch[1],
              toAgent: name,
              eventType: 'message.received',
              timestamp,
              label: `${receivedMatch[1]} → ${name} (message.received)`,
            });
          }
          const readMatch = content.match(/type=message\.read\s+from=(\S+)/);
          if (readMatch) {
            interactions.push({
              fromAgent: readMatch[1],
              toAgent: name,
              eventType: 'message.read',
              timestamp,
              label: `${readMatch[1]} → ${name} (message.read)`,
            });
          }
        }
      }
    }

    // Sort by real timestamp; stable sort preserves JSONL order for ties
    entries.sort((a, b) => a.timestamp - b.timestamp);

    return { entries, interactions };
  });
}

function extractTextContent(blocks: ContentBlock[]): string {
  return blocks
    .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
    .map(b => b.text)
    .join('\n');
}
