import type { FastifyReply, FastifyRequest } from 'fastify';

export interface DashboardEvent {
  id?: number;
  type: string;
  timestamp: number;
  payload: unknown;
}

export class EventBus {
  private static instance: EventBus | null = null;
  private clients = new Set<FastifyReply>();
  private heartbeatInterval: NodeJS.Timeout | null = null;
  private nextEventId = 1;
  private history: DashboardEvent[] = [];
  private readonly historyLimit = parsePositiveInteger(
    process.env.DASHBOARD_EVENT_REPLAY_LIMIT,
    1000,
  );

  private constructor() {
    this.startHeartbeat();
  }

  static getInstance(): EventBus {
    if (!EventBus.instance) {
      EventBus.instance = new EventBus();
    }
    return EventBus.instance;
  }

  registerClient(request: FastifyRequest, reply: FastifyReply): void {
    reply.raw.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
      'X-Accel-Buffering': 'no', // Disable buffering for Nginx/etc.
    });
    reply.raw.write('\n');
    this.clients.add(reply);

    const lastEventId = parseLastEventId(request.headers['last-event-id']);
    if (lastEventId !== null) {
      this.replayAfter(lastEventId, reply);
    }

    request.raw.on('close', () => {
      this.clients.delete(reply);
    });
  }

  broadcast(type: string, payload: unknown): void {
    const envelope: DashboardEvent = {
      id: this.nextEventId++,
      type,
      timestamp: Date.now(),
      payload,
    };
    this.remember(envelope);

    const eventString = this.formatEvent(envelope);

    const deadClients: FastifyReply[] = [];

    for (const client of this.clients) {
      try {
        if (client.raw.destroyed || client.raw.writableEnded) {
          deadClients.push(client);
          continue;
        }
        client.raw.write(eventString);
      } catch (err) {
        console.error('Error writing to client socket, removing client:', err);
        deadClients.push(client);
      }
    }

    for (const dead of deadClients) {
      this.clients.delete(dead);
      try {
        dead.raw.end();
      } catch { /* ignore */ }
    }
  }

  private startHeartbeat(): void {
    this.heartbeatInterval = setInterval(() => {
      if (this.clients.size === 0) return;

      const deadClients: FastifyReply[] = [];
      const pingMessage = 'event: ping\ndata: "heartbeat"\n\n';

      for (const client of this.clients) {
        try {
          if (client.raw.destroyed || client.raw.writableEnded) {
            deadClients.push(client);
            continue;
          }
          client.raw.write(pingMessage);
        } catch {
          deadClients.push(client);
        }
      }

      for (const dead of deadClients) {
        this.clients.delete(dead);
        try {
          dead.raw.end();
        } catch { /* ignore */ }
      }
    }, 20000);
  }

  shutdown(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
    }
    for (const client of this.clients) {
      try {
        client.raw.end();
      } catch { /* ignore */ }
    }
    this.clients.clear();
  }

  private remember(event: DashboardEvent): void {
    this.history.push(event);
    if (this.history.length > this.historyLimit) {
      this.history.splice(0, this.history.length - this.historyLimit);
    }
  }

  private replayAfter(lastEventId: number, reply: FastifyReply): void {
    const replay = this.history.filter(event => (event.id ?? 0) > lastEventId);
    for (const event of replay) {
      try {
        if (reply.raw.destroyed || reply.raw.writableEnded) return;
        reply.raw.write(this.formatEvent(event));
      } catch {
        return;
      }
    }
  }

  private formatEvent(event: DashboardEvent): string {
    const idLine = event.id === undefined ? '' : `id: ${event.id}\n`;
    const envelopeString = `${idLine}event: dashboard-event\ndata: ${JSON.stringify(event)}\n\n`;

    // For legacy compatibility, send old events if they match.
    if (!['message', 'debug-dump', 'agent-online', 'agent-offline'].includes(event.type)) {
      return envelopeString;
    }

    return `${envelopeString}${idLine}event: ${event.type}\ndata: ${JSON.stringify(event.payload)}\n\n`;
  }
}

function parseLastEventId(value: string | string[] | undefined): number | null {
  const raw = Array.isArray(value) ? value[0] : value;
  if (!raw) return null;
  const parsed = Number.parseInt(raw, 10);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

function parsePositiveInteger(value: string | undefined, fallback: number): number {
  if (!value) return fallback;
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
