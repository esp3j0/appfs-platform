import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { TimelineEntry, MessageRecord, ContentBlock, CrossAgentInteraction } from '../types.js';

export function registerTimelineRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/timeline', async (request) => {
    const query = request.query as { agents?: string };
    const agentNames = (query.agents ?? '').split(',').filter(Boolean).map(decodeURIComponent);

    if (agentNames.length === 0) {
      return { entries: [], interactions: [] };
    }

    const interactions: CrossAgentInteraction[] = [];

    // Build per-agent ordered timelines
    const perAgent: Map<string, TimelineEntry[]> = new Map();

    for (const name of agentNames) {
      const msgs = registry.getMessages(name);
      const entries: TimelineEntry[] = [];
      for (const rec of msgs) {
        const msg = rec.message;
        const content = extractTextContent(msg.blocks);
        const timestamp = msg.timestamp_ms ?? 0;
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
      perAgent.set(name, entries);
    }

    // Merge per-agent timelines by timestamp, preserving JSONL order within
    // each agent (stable for equal timestamps). This is a k-way merge.
    const merged = mergeTimelines(perAgent);

    return { entries: merged, interactions };
  });
}

/**
 * K-way merge of per-agent timelines.
 *
 * Each agent's timeline is already in chronological JSONL order.
 * We merge them by comparing the current head of each list using
 * their timestamp_ms, preserving intra-agent order when timestamps
 * are equal (batch persistence).
 */
function mergeTimelines(perAgent: Map<string, TimelineEntry[]>): TimelineEntry[] {
  // Create index pointers for each agent
  const indices = new Map<string, number>();
  const total = Array.from(perAgent.values()).reduce((s, e) => s + e.length, 0);
  const result: TimelineEntry[] = [];
  result.length = total;

  for (let i = 0; i < total; i++) {
    // Find the agent with the smallest timestamp at its current position
    let bestAgent: string | null = null;
    let bestTs = Infinity;

    for (const [name, entries] of perAgent) {
      const idx = indices.get(name) ?? 0;
      if (idx < entries.length) {
        const ts = entries[idx].timestamp;
        if (ts < bestTs) {
          bestTs = ts;
          bestAgent = name;
        }
      }
    }

    if (bestAgent === null) break;

    const idx = indices.get(bestAgent) ?? 0;
    result[i] = perAgent.get(bestAgent)![idx];
    indices.set(bestAgent, idx + 1);
  }

  return result;
}

function extractTextContent(blocks: ContentBlock[]): string {
  return blocks
    .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
    .map(b => b.text)
    .join('\n');
}
