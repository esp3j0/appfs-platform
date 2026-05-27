import Fastify from 'fastify';
import cors from '@fastify/cors';
import fastifyStatic from '@fastify/static';
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
import { registerModelConfigsRoute } from './routes/model-configs.js';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';
import { terminateChildProcessTree } from './child-process-utils.js';
import { ModelConfigStore } from './model-config-store.js';
const PORT = parseInt(process.env.PORT ?? '3100', 10);
const HOST = process.env.HOST ?? '127.0.0.1';
const DUMP_DIR = process.argv[2] ?? process.env.APPFS_DEBUG_DUMP_DIR ?? '';
// In desktop mode, DUMP_DIR is optional and starts with an empty registry
const dumpDir = DUMP_DIR ? path.resolve(DUMP_DIR) : '';
class AppfsProjectRuntimeController {
    registry;
    processManager;
    activeProcesses = new Map();
    constructor(registry, processManager) {
        this.registry = registry;
        this.processManager = processManager;
    }
    start(projectId) {
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
        return new Promise((resolve, reject) => {
            let resolved = false;
            const child = spawn(cmd, args, {
                cwd: project.projectRoot,
                stdio: ['ignore', 'pipe', 'pipe'],
                env: { ...process.env },
                windowsHide: true,
            });
            child.stdout?.on('data', chunk => {
                console.log(`[ProjectRuntime stdout] ${chunk.toString().trim()}`);
            });
            child.stderr?.on('data', chunk => {
                console.error(`[ProjectRuntime stderr] ${chunk.toString().trim()}`);
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
                }
                else if (project.status === 'running') {
                    project.status = 'stopped';
                }
                if (!resolved) {
                    resolved = true;
                    reject(new Error(`AppFS compose process exited prematurely with code ${code}`));
                }
            });
        });
    }
    async stop(projectId) {
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
            this.activeProcesses.delete(projectId);
            await terminateChildProcessTree(child, {
                label: `project runtime ${projectId}`,
                gracefulTimeoutMs: 3000,
            });
        }
        else {
            project.status = 'stopped';
        }
        return project;
    }
    async shutdown() {
        const projectIds = [...this.activeProcesses.keys()];
        if (projectIds.length === 0) {
            return;
        }
        console.log(`[ProjectRuntime] Shutting down active projects: ${projectIds.join(', ')}`);
        await Promise.allSettled(projectIds.map(projectId => this.stop(projectId)));
    }
    status(projectId) {
        return this.registry.getProject(projectId);
    }
}
function resolvePlatformRoot() {
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
    if (dumpDir) {
        try {
            projectRegistry.registerProject(dumpDir);
            console.log(`[ProjectRegistry] Initial project registered for ${dumpDir}`);
        }
        catch (err) {
            console.warn(`[ProjectRegistry] Initial project register failed for ${dumpDir}: ${err.message}`);
        }
    }
    else {
        console.log('[ProjectRegistry] Starting with an empty project registry (desktop mode)');
    }
    const registry = new AgentRegistry(dumpDir, projectRegistry);
    if (dumpDir) {
        registry.discover();
        console.log(`Discovered ${registry.getAgents().length} agent(s) in ${dumpDir}`);
        for (const agent of registry.getAgents()) {
            console.log(`  - ${agent.name} (${agent.model}, ${agent.messageCount} messages)`);
        }
    }
    else {
        console.log('[AgentRegistry] Empty initial registry path (desktop mode)');
    }
    const modelConfigStore = new ModelConfigStore();
    const processManager = new AgentProcessManager(registry, modelConfigStore);
    const runtimeController = new AppfsProjectRuntimeController(projectRegistry, processManager);
    const app = Fastify({ logger: false });
    if (process.env.ELECTRON_RUN_AS_NODE === '1') {
        await app.register(cors, { origin: false });
    }
    else {
        await app.register(cors, {
            origin: ['http://localhost:5173', 'http://127.0.0.1:5173']
        });
    }
    registerAgentsRoute(app, registry);
    registerMessagesRoute(app, registry);
    registerTimelineRoute(app, registry);
    registerEventsRoute(app, registry);
    registerAppEventOverridesRoute(app, registry);
    registerMountedAppsRoute(app, registry, projectRegistry);
    registerModelConfigsRoute(app, modelConfigStore);
    registerProcessRoute(app, processManager);
    registerPrincipalsRoute(app, registry, processManager);
    registerProjectsRoute(app, projectRegistry, runtimeController, {
        agentRegistry: registry,
        processManager,
    });
    let shuttingDown = false;
    const shutdownHandler = async () => {
        if (shuttingDown) {
            return;
        }
        shuttingDown = true;
        console.log('\n[Server] Shutting down...');
        await runtimeController.shutdown();
        await processManager.shutdown();
        await app.close();
        process.exit(0);
    };
    const shutdownToken = process.env.DASHBOARD_SHUTDOWN_TOKEN;
    if (shutdownToken) {
        app.post('/api/admin/shutdown', async (request, reply) => {
            const header = request.headers['x-appfs-shutdown-token'];
            const token = Array.isArray(header) ? header[0] : header;
            if (token !== shutdownToken) {
                return reply.status(403).send({ error: 'Forbidden' });
            }
            reply.status(202).send({ status: 'shutting-down' });
            setImmediate(() => {
                void shutdownHandler();
            });
        });
    }
    // Serve static files from dashboard/dist in packaged production mode
    const moduleDir = path.dirname(fileURLToPath(import.meta.url));
    let distDir = path.resolve(moduleDir, '..', '..', 'dist');
    if (process.env.ELECTRON_RUN_AS_NODE === '1') {
        distDir = distDir.replace('app.asar.unpacked', 'app.asar');
    }
    if (fs.existsSync(distDir)) {
        console.log(`[Server] Serving static dashboard from ${distDir}`);
        await app.register(fastifyStatic, {
            root: distDir,
            prefix: '/',
            wildcard: false,
        });
        // Fallback handler for client SPA routing (index.html fallback)
        app.setNotFoundHandler(async (request, reply) => {
            if (request.url.startsWith('/api')) {
                reply.code(404).send({ error: 'Not Found' });
                return;
            }
            return reply.sendFile('index.html');
        });
    }
    else {
        console.log(`[Server] Static dashboard directory not found at ${distDir}. Running in API-only mode.`);
    }
    // Graceful shutdown
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
