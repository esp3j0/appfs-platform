import fs from 'node:fs';
import path from 'node:path';
import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';

export function registerMountedAppsRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/mounted-apps', async () => {
    const file = path.join(registry.dumpDirectory, '_appfs', 'apps.registry.json');
    if (!fs.existsSync(file)) {
      return { version: 1, apps: [] };
    }
    try {
      const content = fs.readFileSync(file, 'utf-8');
      return JSON.parse(content);
    } catch {
      return { version: 1, apps: [] };
    }
  });
}
