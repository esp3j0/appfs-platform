export class EventBus {
    static instance = null;
    clients = new Set();
    heartbeatInterval = null;
    constructor() {
        this.startHeartbeat();
    }
    static getInstance() {
        if (!EventBus.instance) {
            EventBus.instance = new EventBus();
        }
        return EventBus.instance;
    }
    registerClient(request, reply) {
        reply.raw.writeHead(200, {
            'Content-Type': 'text/event-stream',
            'Cache-Control': 'no-cache',
            Connection: 'keep-alive',
            'X-Accel-Buffering': 'no', // Disable buffering for Nginx/etc.
        });
        reply.raw.write('\n');
        this.clients.add(reply);
        request.raw.on('close', () => {
            this.clients.delete(reply);
        });
    }
    broadcast(type, payload) {
        const envelope = {
            type,
            timestamp: Date.now(),
            payload,
        };
        const envelopeString = `event: dashboard-event\ndata: ${JSON.stringify(envelope)}\n\n`;
        // For legacy compatibility, send old events if they match
        let legacyString = '';
        if (['message', 'debug-dump', 'agent-online', 'agent-offline'].includes(type)) {
            legacyString = `event: ${type}\ndata: ${JSON.stringify(payload)}\n\n`;
        }
        const deadClients = [];
        for (const client of this.clients) {
            try {
                let success = true;
                // First send the standard envelope
                success = client.raw.write(envelopeString);
                // Then send legacy event if applicable
                if (success && legacyString) {
                    success = client.raw.write(legacyString);
                }
                if (!success) {
                    deadClients.push(client);
                }
            }
            catch (err) {
                console.error('Error writing to client socket, removing client:', err);
                deadClients.push(client);
            }
        }
        for (const dead of deadClients) {
            this.clients.delete(dead);
            try {
                dead.raw.end();
            }
            catch { /* ignore */ }
        }
    }
    startHeartbeat() {
        this.heartbeatInterval = setInterval(() => {
            if (this.clients.size === 0)
                return;
            const deadClients = [];
            const pingMessage = 'event: ping\ndata: "heartbeat"\n\n';
            for (const client of this.clients) {
                try {
                    const success = client.raw.write(pingMessage);
                    if (!success) {
                        deadClients.push(client);
                    }
                }
                catch {
                    deadClients.push(client);
                }
            }
            for (const dead of deadClients) {
                this.clients.delete(dead);
                try {
                    dead.raw.end();
                }
                catch { /* ignore */ }
            }
        }, 20000);
    }
    shutdown() {
        if (this.heartbeatInterval) {
            clearInterval(this.heartbeatInterval);
            this.heartbeatInterval = null;
        }
        for (const client of this.clients) {
            try {
                client.raw.end();
            }
            catch { /* ignore */ }
        }
        this.clients.clear();
    }
}
