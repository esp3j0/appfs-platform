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
import { registerPrincipalsRoute } from './routes/principals.js';
import { ProjectRegistry } from './project-registry.js';
import { registerProjectsRoute } from './routes/projects.js';
import type { ProjectRuntimeController } from './routes/projects.js';
import type { ProjectRecord } from './project-registry.js';
import { spawn, ChildProcess } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';

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

class AppfsProjectRuntimeController implements ProjectRuntimeController {
  private activeProcesses = new Map<string, ChildProcess>();

  constructor(
    private registry: ProjectRegistry,
    private processManager: AgentProcessManager
  ) {}

  start(projectId: string): Promise<ProjectRecord> {
    const project = this.registry.getProject(projectId);
    if (!project) {
      return Promise.reject(new Error(`Project ${projectId} not found`));
    }

    if (this.activeProcesses.has(projectId)) {
      project.status = 'running';
      return Promise.resolve(project);
    }

    const platformRoot = resolvePlatformRoot();
    const composeFile = project.composeFilePath;

    let cmd = 'cargo';
    let args = [
      'run',
      '--manifest-path',
      path.join(platformRoot, 'appfs', 'cli', 'Cargo.toml'),
      '--',
      'appfs',
      'compose',
      'up',
      '-f',
      composeFile
    ];

    if (process.env.APPFS_CLI_BIN) {
      cmd = process.env.APPFS_CLI_BIN;
      args = ['appfs', 'compose', 'up', '-f', composeFile];
    }

    console.log(`[ProjectRuntime] Starting AppFS for project ${projectId}: ${cmd} ${args.join(' ')}`);

    return new Promise<ProjectRecord>((resolve, reject) => {
      let resolved = false;

      const child = spawn(cmd, args, {
        cwd: project.projectRoot,
        stdio: 'inherit',
        env: { ...process.env },
      });

      this.activeProcesses.set(projectId, child);

      child.on('spawn', () => {
        if (!resolved) {
          resolved = true;
          project.status = 'running';
          resolve(project);
        }
      });

      child.on('error', (err) => {
        console.error(`[ProjectRuntime] AppFS spawn error for project ${projectId}:`, err);
        this.activeProcesses.delete(projectId);
        project.status = 'error';
        if (!resolved) {
          resolved = true;
          reject(err);
        }
      });

      child.on('exit', (code, signal) => {
        console.log(`[ProjectRuntime] AppFS compose process for project ${projectId} exited with code=${code}, signal=${signal}`);
        this.activeProcesses.delete(projectId);
        
        if (project.status === 'starting') {
          project.status = 'error';
        } else if (project.status === 'running') {
          project.status = 'stopped';
        }
        
        if (!resolved) {
          resolved = true;
          reject(new Error(`AppFS compose process exited prematurely with code ${code}`));
        }
      });
    });
  }

  async stop(projectId: string): Promise<ProjectRecord> {
    const project = this.registry.getProject(projectId);
    if (!project) {
      throw new Error(`Project ${projectId} not found`);
    }

    const managedAgentSessionIds = [...project.managedAgentSessionIds];
    console.log(`[ProjectRuntime] Stopping managed agents for project ${projectId}:`, managedAgentSessionIds);
    for (const sessionId of managedAgentSessionIds) {
      this.processManager.stop(sessionId);
    }

    const child = this.activeProcesses.get(projectId);
    if (child) {
      console.log(`[ProjectRuntime] Stopping AppFS compose process for project ${projectId} (PID: ${child.pid})`);
      project.status = 'stopped';
      child.kill('SIGTERM');
      setTimeout(() => {
        if (!child.killed) {
          child.kill('SIGKILL');
        }
      }, 3000);
      this.activeProcesses.delete(projectId);
    } else {
      project.status = 'stopped';
    }

    return project;
  }

  status(projectId: string): ProjectRecord | undefined {
    return this.registry.getProject(projectId);
  }
}

function resolvePlatformRoot(): string {
  const moduleDir = path.dirname(fileURLToPath(import.meta.url));
  const candidates = [
    path.resolve(moduleDir, '..', '..', '..'),
    path.resolve(process.cwd(), '..', '..'),
    process.cwd(),
  ];
  for (const candidate of candidates) {
    if (fs.existsSync(path.join(candidate, 'appfs-agent', 'rust', 'Cargo.toml'))) {
      return candidate;
    }
  }
  return candidates[0];
}

async function main() {
  const projectRegistry = new ProjectRegistry();
  try {
    projectRegistry.registerProject(dumpDir);
    console.log(`[ProjectRegistry] Initial project registered for ${dumpDir}`);
  } catch (err: any) {
    console.warn(`[ProjectRegistry] Initial project register failed for ${dumpDir}: ${err.message}`);
  }

  const registry = new AgentRegistry(dumpDir, projectRegistry);
  registry.discover();

  console.log(`Discovered ${registry.getAgents().length} agent(s) in ${dumpDir}`);
  for (const agent of registry.getAgents()) {
    console.log(`  - ${agent.name} (${agent.model}, ${agent.messageCount} messages)`);
  }

  const processManager = new AgentProcessManager(registry);
  const runtimeController = new AppfsProjectRuntimeController(projectRegistry, processManager);

  const app = Fastify({ logger: false });
  await app.register(cors, { origin: true });

  registerAgentsRoute(app, registry);
  registerMessagesRoute(app, registry);
  registerTimelineRoute(app, registry);
  registerEventsRoute(app, registry);
  registerAppEventOverridesRoute(app, registry);
  registerMountedAppsRoute(app, registry);
  registerProcessRoute(app, processManager);
  registerPrincipalsRoute(app, registry, processManager);
  registerProjectsRoute(app, projectRegistry, runtimeController);

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
