import { spawn } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface } from 'node:readline';
import { EventBus } from './event-bus.js';
import { terminateChildProcessTree } from './child-process-utils.js';
export function resolveProjectScopedSpawnConfig(spawnConfig, projectRegistry) {
    const resolvedConfig = { ...spawnConfig };
    if (resolvedConfig.projectId && projectRegistry) {
        const project = projectRegistry.getProject(resolvedConfig.projectId);
        if (!project) {
            throw new SpawnConfigValidationError(`Project ${resolvedConfig.projectId} not found for spawn`);
        }
        resolvedConfig.cwd = project.projectRoot;
        resolvedConfig.appfsMountRoot = project.mountRoot;
        resolvedConfig.projectRoot = project.projectRoot;
    }
    return resolvedConfig;
}
// ── AgentProcessManager ──
export class AgentProcessManager {
    modelConfigStore;
    agents = new Map();
    eventBus;
    registry;
    /**
     * Maps spawn-time placeholder IDs to actual sessionIds returned by
     * the agent's `session_started` event. Allows us to track agents
     * before we know their real sessionId.
     */
    pendingSpawnMap = new Map();
    constructor(registry, modelConfigStore) {
        this.modelConfigStore = modelConfigStore;
        this.eventBus = EventBus.getInstance();
        this.registry = registry;
    }
    // ── Spawn ──
    spawn(spawnConfig) {
        const spawnId = `spawn-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        const effectiveSpawnConfig = resolveProjectScopedSpawnConfig(spawnConfig, this.registry.projectRegistry);
        this.assertSpawnConfig(effectiveSpawnConfig);
        this.resolveRuntimeModelConfig(effectiveSpawnConfig, spawnId);
        const args = this.buildArgs(effectiveSpawnConfig);
        const cmd = this.buildCommand(effectiveSpawnConfig.launchSpec);
        console.log(`[ProcessManager] Spawning agent ${spawnId}: ${cmd} ${args.join(' ')}`);
        this.eventBus.broadcast('process-log', {
            agentId: spawnId,
            spawnId,
            stream: 'spawn',
            text: `Spawning agent ${spawnId}: ${cmd} ${args.join(' ')}`,
        });
        const childProcess = spawn(cmd, args, {
            cwd: effectiveSpawnConfig.cwd,
            stdio: ['pipe', 'pipe', 'pipe'],
            env: this.buildEnvironment(effectiveSpawnConfig),
            shell: false,
            windowsHide: true,
        });
        const stdoutReader = createInterface({ input: childProcess.stdout });
        const stderrReader = createInterface({ input: childProcess.stderr });
        const managedAgent = {
            process: childProcess,
            sessionId: null,
            spawnConfig: effectiveSpawnConfig,
            status: 'starting',
            currentRequestId: null,
            controlEndpoint: null,
            stdoutReader,
            stderrReader,
        };
        this.agents.set(spawnId, managedAgent);
        // ── stdout JSONL line parser ──
        stdoutReader.on('line', (line) => {
            this.handleStdoutLine(spawnId, line);
        });
        // ── stderr log forwarder ──
        stderrReader.on('line', (line) => {
            const agentId = managedAgent.sessionId ?? spawnId;
            this.eventBus.broadcast('process-log', {
                agentId,
                spawnId,
                stream: 'stderr',
                text: line,
            });
        });
        // ── Process exit ──
        childProcess.on('exit', (code, signal) => {
            const agentId = managedAgent.sessionId ?? spawnId;
            console.log(`[ProcessManager] Agent ${agentId} exited with code=${code}, signal=${signal}`);
            this.eventBus.broadcast('agent-offline', {
                sessionId: agentId,
                spawnId,
                code,
                signal,
            });
            // Update registry status if we have a real sessionId
            if (managedAgent.sessionId) {
                const existingAgent = this.registry.getAgent(managedAgent.sessionId);
                if (existingAgent) {
                    this.registry.registerAgent({ ...existingAgent, status: 'offline' });
                }
            }
            // Clean up
            stdoutReader.close();
            stderrReader.close();
            childProcess.stdout?.destroy();
            childProcess.stderr?.destroy();
            childProcess.stdin?.end();
            childProcess.stdin?.destroy();
            this.agents.delete(spawnId);
            if (managedAgent.sessionId) {
                this.pendingSpawnMap.delete(managedAgent.sessionId);
            }
        });
        childProcess.on('error', (err) => {
            console.error(`[ProcessManager] Spawn error for ${spawnId}:`, err);
            this.eventBus.broadcast('process-log', {
                agentId: spawnId,
                spawnId,
                stream: 'error',
                text: `Spawn error: ${err.message}`,
            });
            stdoutReader.close();
            stderrReader.close();
            childProcess.stdout?.destroy();
            childProcess.stderr?.destroy();
            childProcess.stdin?.end();
            childProcess.stdin?.destroy();
            this.agents.delete(spawnId);
        });
        return { spawnId };
    }
    // ── Prompt submission ──
    /**
     * Send a prompt to an agent identified by sessionId.
     * Returns request_id on success; throws on error.
     */
    async sendPrompt(sessionId, promptText, delivery = 'prompt') {
        const managed = this.findBySessionId(sessionId);
        if (!managed) {
            throw new Error(`No managed agent found for sessionId: ${sessionId}`);
        }
        if (managed.status === 'starting') {
            throw new Error(`Agent ${sessionId} is still starting up`);
        }
        const requestId = `req-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        const isBusy = managed.status === 'busy';
        const effectiveDelivery = isBusy
            ? (delivery === 'guidance' ? 'guidance' : 'queue')
            : 'prompt';
        const inputType = effectiveDelivery === 'guidance'
            ? 'user_guidance'
            : effectiveDelivery === 'queue'
                ? 'user_queued'
                : 'user_prompt';
        if (inputType === 'user_prompt') {
            managed.status = 'busy';
            managed.currentRequestId = requestId;
        }
        try {
            await this.writeControlInput(managed, {
                type: inputType,
                request_id: requestId,
                text: promptText,
            });
        }
        catch (error) {
            if (inputType === 'user_prompt' && managed.currentRequestId === requestId) {
                managed.status = 'idle';
                managed.currentRequestId = null;
            }
            throw error;
        }
        if (inputType === 'user_prompt') {
            return { requestId, status: 'accepted' };
        }
        return {
            requestId,
            status: inputType === 'user_guidance' ? 'guidance' : 'queued',
        };
    }
    async promoteQueuedInput(sessionId, requestId) {
        const managed = this.findBySessionId(sessionId);
        if (!managed) {
            throw new Error(`No managed agent found for sessionId: ${sessionId}`);
        }
        if (managed.status === 'starting') {
            throw new Error(`Agent ${sessionId} is still starting up`);
        }
        await this.writeControlInput(managed, { type: 'promote_input', request_id: requestId });
        return { requestId, status: 'guidance' };
    }
    async cancelTurn(sessionId, requestId) {
        const managed = this.findBySessionId(sessionId);
        if (!managed) {
            throw new Error(`No managed agent found for sessionId: ${sessionId}`);
        }
        if (managed.status === 'starting') {
            throw new Error(`Agent ${sessionId} is still starting up`);
        }
        const activeRequestId = requestId?.trim() || managed.currentRequestId;
        if (!activeRequestId || managed.status !== 'busy') {
            throw new AgentNoActiveTurnError(sessionId);
        }
        await this.writeControlInput(managed, {
            type: 'cancel_turn',
            request_id: activeRequestId,
        });
        return { requestId: activeRequestId, status: 'cancelling' };
    }
    // ── Stop agent ──
    stop(sessionId) {
        const managed = this.findBySessionId(sessionId);
        if (!managed)
            return false;
        void terminateChildProcessTree(managed.process, {
            label: `agent ${managed.sessionId ?? sessionId}`,
            gracefulTimeoutMs: 5000,
        });
        return true;
    }
    // ── Status ──
    getStatus(sessionId) {
        const managed = this.findBySessionId(sessionId);
        if (!managed)
            return null;
        return { status: managed.status, currentRequestId: managed.currentRequestId };
    }
    getManagedSessionIds() {
        return Array.from(this.agents.values())
            .filter(a => a.sessionId !== null)
            .map(a => a.sessionId);
    }
    resumeProjectAgents(projectId) {
        const result = {
            resumed: [],
            skipped: [],
            errors: [],
        };
        const project = this.registry.projectRegistry.getProject(projectId);
        if (!project) {
            result.errors.push({ sessionId: projectId, error: `Project ${projectId} not found` });
            return result;
        }
        const agents = this.registry.getAgents()
            .filter(agent => agent.projectId === projectId);
        for (const agent of agents) {
            if (!agent.sessionJsonlPath) {
                result.skipped.push({ sessionId: agent.sessionId, reason: 'missing session path' });
                continue;
            }
            if (this.findBySessionId(agent.sessionId)) {
                result.skipped.push({ sessionId: agent.sessionId, reason: 'already managed' });
                continue;
            }
            if (this.findBySessionPath(agent.sessionJsonlPath)) {
                result.skipped.push({ sessionId: agent.sessionId, reason: 'already starting' });
                continue;
            }
            if (agent.status === 'online') {
                result.skipped.push({ sessionId: agent.sessionId, reason: 'already online' });
                continue;
            }
            try {
                const base = this.getDefaultSpawnConfig();
                const model = agent.model && agent.model !== 'unknown'
                    ? agent.model
                    : base.model;
                const { spawnId } = this.spawn({
                    ...base,
                    principalId: agent.principalId || agent.name,
                    model,
                    sessionPath: agent.sessionJsonlPath,
                    projectId: project.projectId,
                    projectRoot: project.projectRoot,
                    cwd: project.projectRoot,
                    appfsMountRoot: project.mountRoot,
                });
                result.resumed.push({ sessionId: agent.sessionId, spawnId });
            }
            catch (err) {
                result.errors.push({
                    sessionId: agent.sessionId,
                    error: err instanceof Error ? err.message : String(err),
                });
            }
        }
        return result;
    }
    getManagedAgents() {
        return Array.from(this.agents.values()).map(a => ({
            pid: a.process.pid,
            sessionId: a.sessionId,
            status: a.status,
            principalId: a.spawnConfig.principalId,
            model: a.spawnConfig.model,
            permissionMode: a.spawnConfig.permissionMode,
        }));
    }
    getDefaultSpawnConfig() {
        const platformRoot = resolvePlatformRoot();
        if (process.env.DASHBOARD_AGENT_BIN) {
            return {
                cwd: this.registry.dumpDirectory,
                principalId: 'default',
                model: process.env.DASHBOARD_AGENT_MODEL ?? 'claude-opus-4-6',
                permissionMode: process.env.DASHBOARD_AGENT_PERMISSION_MODE ?? 'dangerous',
                appfsMountRoot: this.registry.dumpDirectory,
                appfsIdleWake: true,
                env: {},
                launchSpec: {
                    kind: 'binary',
                    binaryPath: process.env.DASHBOARD_AGENT_BIN,
                },
            };
        }
        return {
            cwd: this.registry.dumpDirectory,
            principalId: 'default',
            model: process.env.DASHBOARD_AGENT_MODEL ?? 'claude-opus-4-6',
            permissionMode: process.env.DASHBOARD_AGENT_PERMISSION_MODE ?? 'dangerous',
            appfsMountRoot: this.registry.dumpDirectory,
            appfsIdleWake: true,
            env: {},
            launchSpec: {
                kind: 'cargo',
                manifestPath: process.env.DASHBOARD_AGENT_MANIFEST
                    ?? path.join(platformRoot, 'appfs-agent', 'rust', 'Cargo.toml'),
                targetDir: process.env.DASHBOARD_AGENT_TARGET_DIR
                    ?? path.join(os.tmpdir(), 'appfs-agent-local-target'),
                package: 'rusty-claude-cli',
                features: ['debug-dump'],
            },
        };
    }
    // ── Private helpers ──
    findBySessionId(sessionId) {
        for (const agent of this.agents.values()) {
            if (agent.sessionId === sessionId)
                return agent;
        }
        return null;
    }
    findBySessionPath(sessionPath) {
        const target = path.resolve(sessionPath);
        for (const agent of this.agents.values()) {
            if (agent.spawnConfig.sessionPath && path.resolve(agent.spawnConfig.sessionPath) === target) {
                return agent;
            }
        }
        return null;
    }
    assertSpawnConfig(config) {
        const missing = [];
        if (!config.cwd?.trim())
            missing.push('cwd');
        if (!config.principalId?.trim())
            missing.push('principalId');
        if (!config.model?.trim())
            missing.push('model');
        if (!config.appfsMountRoot?.trim())
            missing.push('appfsMountRoot');
        if (!config.launchSpec) {
            missing.push('launchSpec');
        }
        else if (config.launchSpec.kind === 'binary') {
            if (!config.launchSpec.binaryPath?.trim())
                missing.push('launchSpec.binaryPath');
        }
        else if (config.launchSpec.kind === 'cargo') {
            if (!config.launchSpec.manifestPath?.trim())
                missing.push('launchSpec.manifestPath');
            if (!config.launchSpec.package?.trim())
                missing.push('launchSpec.package');
        }
        if (missing.length > 0) {
            throw new SpawnConfigValidationError(`Missing required fields: ${missing.join(', ')}`);
        }
    }
    writeControlInput(managed, payload) {
        const endpoint = managed.controlEndpoint;
        if (!endpoint) {
            throw new Error(`Agent ${managed.sessionId ?? 'starting'} has not published a headless control endpoint`);
        }
        if (endpoint.kind !== 'tcp_jsonl') {
            throw new Error(`Unsupported headless control endpoint kind: ${endpoint.kind}`);
        }
        const line = JSON.stringify({
            ...payload,
            control_token: endpoint.token,
        }) + '\n';
        return new Promise((resolve, reject) => {
            let settled = false;
            const socket = net.createConnection({ host: endpoint.host, port: endpoint.port }, () => {
                socket.end(line, 'utf8');
            });
            const settle = (error) => {
                if (settled)
                    return;
                settled = true;
                socket.removeAllListeners();
                if (error) {
                    reject(error);
                }
                else {
                    resolve();
                }
            };
            socket.on('error', (error) => settle(error));
            socket.on('close', (hadError) => {
                if (!hadError)
                    settle();
            });
            socket.setTimeout(2000, () => {
                socket.destroy();
                settle(new Error('Timed out while writing to headless control endpoint'));
            });
        });
    }
    handleStdoutLine(spawnId, line) {
        const managed = this.agents.get(spawnId);
        if (!managed)
            return;
        let event;
        try {
            event = JSON.parse(line);
        }
        catch {
            // Non-JSON line from stdout; log as process output
            this.eventBus.broadcast('process-log', {
                agentId: managed.sessionId ?? spawnId,
                spawnId,
                stream: 'stdout',
                text: line,
            });
            return;
        }
        const agentId = managed.sessionId ?? spawnId;
        switch (event.type) {
            case 'session_started': {
                const sessionId = event.session_id;
                if (sessionId) {
                    managed.sessionId = sessionId;
                    managed.status = 'idle';
                    managed.controlEndpoint = event.control ?? null;
                    this.pendingSpawnMap.set(sessionId, spawnId);
                    // Register in the agent registry as a managed agent
                    const sessionJsonlPath = event.session_path ?? '';
                    const principalId = event.principal_id ?? managed.spawnConfig.principalId;
                    const agentInfo = {
                        name: principalId,
                        principalId,
                        sessionId,
                        workspaceFingerprint: workspaceFingerprintFromSessionPath(sessionJsonlPath),
                        model: managed.spawnConfig.model,
                        pid: managed.process.pid ?? 0,
                        startedAt: Date.now(),
                        sessionJsonlPath,
                        status: 'online',
                        controlMode: 'managed',
                        messageCount: 0,
                        totalInputTokens: 0,
                        totalOutputTokens: 0,
                        projectId: managed.spawnConfig.projectId,
                        projectRoot: managed.spawnConfig.projectRoot,
                    };
                    this.registry.registerAgent(agentInfo);
                    console.log(`[ProcessManager] Agent ${spawnId} started with sessionId=${sessionId}`);
                    this.eventBus.broadcast('process-log', {
                        agentId: sessionId,
                        spawnId,
                        stream: 'stdout',
                        text: `session_started sessionId=${sessionId} principal=${principalId} sessionPath=${sessionJsonlPath || '<none>'}`,
                    });
                    this.eventBus.broadcast('agent-online', {
                        ...agentInfo,
                        spawnId,
                    });
                }
                else {
                    this.eventBus.broadcast('agent-online', {
                        sessionId: spawnId,
                        spawnId,
                        controlMode: 'managed',
                    });
                }
                break;
            }
            case 'turn_start': {
                managed.status = 'busy';
                managed.currentRequestId = event.request_id ?? null;
                this.eventBus.broadcast('turn-start', {
                    sessionId: agentId,
                    requestId: event.request_id,
                    turnId: event.turn_id,
                });
                break;
            }
            case 'assistant_delta': {
                this.eventBus.broadcast('assistant-delta', {
                    sessionId: agentId,
                    requestId: event.request_id,
                    turnId: event.turn_id,
                    text: event.text,
                });
                break;
            }
            case 'tool_start': {
                this.eventBus.broadcast('tool-start', {
                    sessionId: agentId,
                    requestId: event.request_id,
                    turnId: event.turn_id,
                    id: event.id,
                    toolName: event.tool_name,
                });
                break;
            }
            case 'tool_result': {
                this.eventBus.broadcast('tool-result', {
                    sessionId: agentId,
                    requestId: event.request_id,
                    turnId: event.turn_id,
                    id: event.id,
                    toolName: event.tool_name,
                    isError: event.is_error,
                });
                break;
            }
            case 'turn_done': {
                managed.status = 'idle';
                managed.currentRequestId = null;
                this.eventBus.broadcast('turn-done', {
                    sessionId: agentId,
                    requestId: event.request_id,
                    turnId: event.turn_id,
                    status: event.status,
                    usage: event.usage,
                });
                break;
            }
            case 'error': {
                // If it was busy, transition back to idle
                if (managed.status === 'busy') {
                    managed.status = 'idle';
                    managed.currentRequestId = null;
                }
                this.eventBus.broadcast('agent-error', {
                    sessionId: agentId,
                    requestId: event.request_id,
                    turnId: event.turn_id,
                    message: event.message,
                });
                break;
            }
            default: {
                // Forward unknown events verbatim
                this.eventBus.broadcast('headless-event', {
                    sessionId: agentId,
                    event,
                });
                break;
            }
        }
    }
    buildCommand(launchSpec) {
        if (launchSpec.kind === 'binary') {
            return launchSpec.binaryPath;
        }
        return 'cargo';
    }
    buildArgs(config) {
        const spec = config.launchSpec;
        if (spec.kind === 'cargo') {
            const args = ['run', '--manifest-path', spec.manifestPath];
            if (spec.targetDir) {
                args.push('--target-dir', spec.targetDir);
            }
            args.push('-p', spec.package);
            if (spec.features && spec.features.length > 0) {
                args.push('--features', spec.features.join(','));
            }
            args.push('--');
            // Headless flags
            if (config.model.trim()) {
                args.push('--model', config.model.trim());
            }
            if (config.runtimeModelConfigPath?.trim()) {
                args.push('--model-config', config.runtimeModelConfigPath.trim());
            }
            args.push(...this.permissionArgs(config.permissionMode));
            args.push('--headless');
            if (config.appfsIdleWake) {
                args.push('--appfs-idle-wake');
            }
            if (config.sessionPath?.trim()) {
                args.push('--session', config.sessionPath.trim());
            }
            return args;
        }
        // Binary launch
        const args = ['--headless'];
        if (config.model.trim()) {
            args.push('--model', config.model.trim());
        }
        if (config.runtimeModelConfigPath?.trim()) {
            args.push('--model-config', config.runtimeModelConfigPath.trim());
        }
        args.push(...this.permissionArgs(config.permissionMode));
        if (config.appfsIdleWake) {
            args.push('--appfs-idle-wake');
        }
        if (config.sessionPath?.trim()) {
            args.push('--session', config.sessionPath.trim());
        }
        return args;
    }
    buildEnvironment(config) {
        const mountRoot = path.resolve(config.appfsMountRoot);
        return {
            ...process.env,
            ...config.env,
            APPFS_PRINCIPAL_ID: config.principalId,
            APPFS_MOUNT_ROOT: mountRoot,
            APPFS_RUNTIME_MANIFEST: path.join(mountRoot, '.well-known', 'appfs', 'runtime.json'),
        };
    }
    resolveRuntimeModelConfig(config, spawnId) {
        if (config.runtimeModelConfigPath || !this.modelConfigStore) {
            return;
        }
        const resolved = this.modelConfigStore.resolveSelection({
            providerId: config.modelProviderId,
            modelId: config.modelId,
            modelName: config.model,
            contextWindowTokens: config.contextWindowTokens,
            maxOutputTokens: config.maxOutputTokens,
        });
        config.model = resolved.model.name;
        config.modelProviderId = resolved.providerId;
        config.modelId = resolved.modelId;
        config.contextWindowTokens = resolved.model.contextWindowTokens;
        config.maxOutputTokens = resolved.model.maxOutputTokens;
        config.runtimeModelConfigPath = this.modelConfigStore.writeRuntimeConfig(resolved, spawnId);
    }
    permissionArgs(permissionMode) {
        const normalized = permissionMode.trim();
        if (!normalized || normalized === 'default') {
            return [];
        }
        if (normalized === 'dangerous' || normalized === 'danger-full-access') {
            return ['--dangerously-skip-permissions'];
        }
        if (normalized === 'read-only' || normalized === 'workspace-write') {
            return ['--permission-mode', normalized];
        }
        return ['--permission-mode', normalized];
    }
    // ── Shutdown all ──
    async shutdown() {
        const shutdowns = Array.from(this.agents.entries()).map(([spawnId, managed]) => {
            console.log(`[ProcessManager] Shutting down agent ${managed.sessionId ?? spawnId}`);
            return terminateChildProcessTree(managed.process, {
                label: `agent ${managed.sessionId ?? spawnId}`,
                gracefulTimeoutMs: 5000,
            });
        });
        await Promise.allSettled(shutdowns);
        this.agents.clear();
    }
}
function workspaceFingerprintFromSessionPath(sessionPath) {
    if (!sessionPath)
        return undefined;
    const normalized = sessionPath.replace(/\\/g, '/');
    const parts = normalized.split('/').filter(Boolean);
    const index = parts.lastIndexOf('sessions');
    if (index >= 0 && parts.length > index + 1) {
        return parts[index + 1];
    }
    return undefined;
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
// ── Error types ──
export class AgentBusyError extends Error {
    constructor(sessionId) {
        super(`Agent ${sessionId} is currently busy with an active turn`);
        this.name = 'AgentBusyError';
    }
}
export class AgentNoActiveTurnError extends Error {
    constructor(sessionId) {
        super(`Agent ${sessionId} does not have an active turn to cancel`);
        this.name = 'AgentNoActiveTurnError';
    }
}
export class SpawnConfigValidationError extends Error {
    constructor(message) {
        super(message);
        this.name = 'SpawnConfigValidationError';
    }
}
