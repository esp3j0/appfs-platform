import { FileWatcher } from '../file-watcher.js';
import { EventBus } from '../event-bus.js';
export function registerEventsRoute(app, registry) {
    const eventBus = EventBus.getInstance();
    const watcher = new FileWatcher(registry);
    // Register watcher on the registry so other components can dynamically add paths to it
    registry.setFileWatcher(watcher);
    watcher.start((sessionId, newRecords) => {
        const agent = registry.getAgent(sessionId);
        const agentName = agent?.name ?? sessionId;
        for (const rec of newRecords) {
            const msg = rec.message;
            const entry = {
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
        await new Promise(() => { });
    });
}
function extractTextContent(blocks) {
    return blocks
        .filter((b) => b.type === 'text')
        .map(b => b.text)
        .join('\n');
}
