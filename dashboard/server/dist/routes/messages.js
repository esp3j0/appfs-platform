import { normalizeChatThread } from '../chat-normalizer.js';
export function registerMessagesRoute(app, registry) {
    app.get('/api/agents/:name/messages', async (request) => {
        const { name } = request.params;
        const messages = registry.getMessages(decodeURIComponent(name));
        return messages;
    });
    app.get('/api/agents/:sessionId/chat', async (request) => {
        const { sessionId } = request.params;
        const decodedSessionId = decodeURIComponent(sessionId);
        return normalizeChatThread(decodedSessionId, registry.getMessages(decodedSessionId));
    });
}
