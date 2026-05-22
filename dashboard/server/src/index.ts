import Fastify from 'fastify';
import cors from '@fastify/cors';
import path from 'node:path';
import { AgentRegistry } from './agent-registry.js';
import { AgentProcessManager } from './process-manager.js';
import { registerAgentsRoute } from './routes/agents.js';
import { registerMessagesRoute } from './routes/messages.js';
import { registerTimelineRoute } from './routes/timeline.js';
import { registerEventsRoute } from './routes/events.js';
import { registerAppEventOverridesRoute } from './routes/app-event-overrides.js';
import { registerMountedAppsRoute } from './routes/mounted-apps.js';
import { registerProcessRoute } from './routes/process.js';

const PORT = parseInt(process.env.PORT ?? '3100', 10);
const HOST = process.env.HOST ?? '127.0.0.1';
const DUMP_DIR = process.argv[2] ?? process.env.APPFS_DEBUG_DUMP_DIR ?? '';

if (!DUMP_DIR) {
  console.error('Usage: tsx src/index.ts <workspace-or-dump-dir>');
  console.error('');
  console.error('  Accepts:');
  console.error('    - AppFS mount point (e.g. C:\\mnt\\appfs-compose-tinode)');
  console.error('      → scans .claw/sessions/<hash>/*.jsonl automatically');
  console.error('    - Flat directory with *.jsonl fixtures');
  console.error('    - Directory with agent-meta-*.json (debug-dump mode)');
  console.error('');
  console.error('  Or: set APPFS_DEBUG_DUMP_DIR=<path>');
  process.exit(1);
}

const dumpDir = path.resolve(DUMP_DIR);

async function main() {
  const registry = new AgentRegistry(dumpDir);
  registry.discover();

  console.log(`Discovered ${registry.getAgents().length} agent(s) in ${dumpDir}`);
  for (const agent of registry.getAgents()) {
    console.log(`  - ${agent.name} (${agent.model}, ${agent.messageCount} messages)`);
  }

  const processManager = new AgentProcessManager(registry);

  const app = Fastify({ logger: false });
  await app.register(cors, { origin: true });

  registerAgentsRoute(app, registry);
  registerMessagesRoute(app, registry);
  registerTimelineRoute(app, registry);
  registerEventsRoute(app, registry);
  registerAppEventOverridesRoute(app, registry);
  registerMountedAppsRoute(app, registry);
  registerProcessRoute(app, processManager);

  // Graceful shutdown
  const shutdownHandler = async () => {
    console.log('\n[Server] Shutting down...');
    await processManager.shutdown();
    await app.close();
    process.exit(0);
  };
  process.on('SIGINT', shutdownHandler);
  process.on('SIGTERM', shutdownHandler);

  await app.listen({ port: PORT, host: HOST });
  console.log(`Dashboard API listening on http://${HOST}:${PORT}`);
  console.log(`Process Manager ready — POST /api/process/spawn to launch headless agents`);
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
