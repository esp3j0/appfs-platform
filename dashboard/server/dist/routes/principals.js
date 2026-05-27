import fs from 'node:fs';
import path from 'node:path';
export function registerPrincipalsRoute(app, registry, processManager) {
    app.get('/api/principals', async () => {
        const file = path.join(registry.dumpDirectory, '_appfs', 'principals.registry.json');
        let registryData = { version: 1, principals: [] };
        if (fs.existsSync(file)) {
            try {
                const content = fs.readFileSync(file, 'utf-8');
                registryData = JSON.parse(content);
            }
            catch (err) {
                console.error('[PrincipalsRoute] Error reading principal registry:', err);
            }
        }
        // Get active headless processes from process manager
        const managedAgents = processManager.getManagedAgents();
        const registryAgents = registry.getAgents();
        // Map registry principals and merge with managed agent info
        const principals = registryData.principals.map(principal => {
            // Find matching managed agent first (managed wins)
            const managedActive = managedAgents.find(a => a.principalId === principal.principal_id);
            // If not managed, find in registry agents (prefer online, fallback to most recently started)
            const registryActive = !managedActive
                ? registryAgents.filter(a => a.principalId === principal.principal_id)
                    .sort((a, b) => {
                    if (a.status === 'online' && b.status !== 'online')
                        return -1;
                    if (a.status !== 'online' && b.status === 'online')
                        return 1;
                    return b.startedAt - a.startedAt;
                })[0]
                : undefined;
            const active = managedActive || registryActive;
            return {
                ...principal,
                online: active ? active.status !== 'offline' : false,
                status: active?.status ?? 'offline',
                pid: active?.pid && active.pid !== 0 ? active.pid : undefined,
                sessionId: active?.sessionId ?? null,
                model: active?.model,
                permissionMode: active && 'permissionMode' in active ? active.permissionMode : undefined,
            };
        });
        return {
            version: registryData.version,
            default_principal_id: registryData.default_principal_id,
            principals,
        };
    });
}
