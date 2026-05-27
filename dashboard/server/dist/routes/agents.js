export function registerAgentsRoute(app, registry) {
    app.get('/api/agents', async () => {
        return registry.getAgents();
    });
}
