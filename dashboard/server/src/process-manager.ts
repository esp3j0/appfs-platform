import { spawn, ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface, Interface as ReadlineInterface } from 'node:readline';
import { EventBus } from './event-bus.js';
import type { AgentRegistry } from './agent-registry.js';
import type { AgentInfo } from './types.js';

// ── Launch specification types ──

export type AgentLaunchSpec =
  | { kind: 'cargo'; manifestPath: string; targetDir?: string; package: string; features?: string[] }
  | { kind: 'binary'; binaryPath: string };

export interface SpawnConfig {
  cwd: string;
  principalId: string;
  model: string;
  permissionMode: string;
  appfsMountRoot: string;
  launchSpec: AgentLaunchSpec;
  env: Record<string, string>;
  appfsIdleWake?: boolean;
  sessionPath?: string;
}

export type PromptDelivery = 'prompt' | 'queue' | 'guidance';

export type PromptSubmissionStatus = 'accepted' | 'queued' | 'guidance';

// ── Headless JSONL protocol event types (from Rust stdout) ──

export interface HeadlessEvent {
  type: string;
  session_id?: string;
  session_path?: string;
  principal_id?: string;
  control?: HeadlessControlEndpoint;
  request_id?: string;
  turn_id?: string;
  id?: string;
  tool_name?: string;
  text?: string;
  is_error?: boolean;
  status?: string;
  message?: string;
  usage?: { input_tokens?: number; output_tokens?: number };
}

interface HeadlessControlEndpoint {
  kind: 'tcp_jsonl';
  host: string;
  port: number;
  token: string;
}

// ── Managed agent state ──

interface ManagedAgent {
  process: ChildProcess;
  sessionId: string | null;   // null until session_started is received
  spawnConfig: SpawnConfig;
  status: 'starting' | 'idle' | 'busy';
  currentRequestId: string | null;
  controlEndpoint: HeadlessControlEndpoint | null;
  stdoutReader: ReadlineInterface;
  stderrReader: ReadlineInterface;
}

type ManagedAgentMap = Map<string, ManagedAgent>;

// ── AgentProcessManager ──

export class AgentProcessManager {
  private agents: ManagedAgentMap = new Map();
  private eventBus: EventBus;
  private registry: AgentRegistry;

  /**
   * Maps spawn-time placeholder IDs to actual sessionIds returned by
   * the agent's `session_started` event. Allows us to track agents
   * before we know their real sessionId.
   */
  private pendingSpawnMap = new Map<string, string>();

  constructor(registry: AgentRegistry) {
    this.eventBus = EventBus.getInstance();
    this.registry = registry;
  }

  // ── Spawn ──

  spawn(spawnConfig: SpawnConfig): { spawnId: string } {
    const spawnId = `spawn-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

    const args = this.buildArgs(spawnConfig);
    const cmd = this.buildCommand(spawnConfig.launchSpec);

    console.log(`[ProcessManager] Spawning agent ${spawnId}: ${cmd} ${args.join(' ')}`);

    const childProcess = spawn(cmd, args, {
      cwd: spawnConfig.cwd,
      stdio: ['pipe', 'pipe', 'pipe'],
      env: this.buildEnvironment(spawnConfig),
      shell: false,
    });

    const stdoutReader = createInterface({ input: childProcess.stdout! });
    const stderrReader = createInterface({ input: childProcess.stderr! });

    const managedAgent: ManagedAgent = {
      process: childProcess,
      sessionId: null,
      spawnConfig,
      status: 'starting',
      currentRequestId: null,
      controlEndpoint: null,
      stdoutReader,
      stderrReader,
    };

    this.agents.set(spawnId, managedAgent);

    // ── stdout JSONL line parser ──
    stdoutReader.on('line', (line: string) => {
      this.handleStdoutLine(spawnId, line);
    });

    // ── stderr log forwarder ──
    stderrReader.on('line', (line: string) => {
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
    });

    return { spawnId };
  }

  // ── Prompt submission ──

  /**
   * Send a prompt to an agent identified by sessionId.
   * Returns request_id on success; throws on error.
   */
  async sendPrompt(
    sessionId: string,
    promptText: string,
    delivery: PromptDelivery = 'prompt',
  ): Promise<{ requestId: string; status: PromptSubmissionStatus }> {
    const managed = this.findBySessionId(sessionId);
    if (!managed) {
      throw new Error(`No managed agent found for sessionId: ${sessionId}`);
    }

    if (managed.status === 'starting') {
      throw new Error(`Agent ${sessionId} is still starting up`);
    }

    const requestId = `req-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const isBusy = managed.status === 'busy';
    const effectiveDelivery: PromptDelivery = isBusy
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
    } catch (error) {
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

  async promoteQueuedInput(sessionId: string, requestId: string): Promise<{ requestId: string; status: 'guidance' }> {
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

  // ── Stop agent ──

  stop(sessionId: string): boolean {
    const managed = this.findBySessionId(sessionId);
    if (!managed) return false;

    // Close stdin to signal graceful exit, then kill after timeout
    managed.process.stdin?.end();

    setTimeout(() => {
      if (!managed.process.killed) {
        managed.process.kill('SIGKILL');
      }
    }, 5000);

    return true;
  }

  // ── Status ──

  getStatus(sessionId: string): { status: string; currentRequestId: string | null } | null {
    const managed = this.findBySessionId(sessionId);
    if (!managed) return null;
    return { status: managed.status, currentRequestId: managed.currentRequestId };
  }

  getManagedSessionIds(): string[] {
    return Array.from(this.agents.values())
      .filter(a => a.sessionId !== null)
      .map(a => a.sessionId!);
  }

  getManagedAgents(): Array<{
    pid?: number;
    sessionId: string | null;
    status: 'starting' | 'idle' | 'busy';
    principalId: string;
    model: string;
    permissionMode: string;
  }> {
    return Array.from(this.agents.values()).map(a => ({
      pid: a.process.pid,
      sessionId: a.sessionId,
      status: a.status,
      principalId: a.spawnConfig.principalId,
      model: a.spawnConfig.model,
      permissionMode: a.spawnConfig.permissionMode,
    }));
  }

  getDefaultSpawnConfig(): SpawnConfig {
    const platformRoot = resolvePlatformRoot();
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

  private findBySessionId(sessionId: string): ManagedAgent | null {
    for (const agent of this.agents.values()) {
      if (agent.sessionId === sessionId) return agent;
    }
    return null;
  }

  private writeControlInput(managed: ManagedAgent, payload: Record<string, unknown>): Promise<void> {
    const endpoint = managed.controlEndpoint;
    if (!endpoint) {
      throw new Error(
        `Agent ${managed.sessionId ?? 'starting'} has not published a headless control endpoint`,
      );
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

      const settle = (error?: Error) => {
        if (settled) return;
        settled = true;
        socket.removeAllListeners();
        if (error) {
          reject(error);
        } else {
          resolve();
        }
      };

      socket.on('error', (error) => settle(error));
      socket.on('close', (hadError) => {
        if (!hadError) settle();
      });
      socket.setTimeout(2000, () => {
        socket.destroy();
        settle(new Error('Timed out while writing to headless control endpoint'));
      });
    });
  }

  private handleStdoutLine(spawnId: string, line: string): void {
    const managed = this.agents.get(spawnId);
    if (!managed) return;

    let event: HeadlessEvent;
    try {
      event = JSON.parse(line);
    } catch {
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
          const agentInfo: AgentInfo = {
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
          };
          this.registry.registerAgent(agentInfo);

          console.log(`[ProcessManager] Agent ${spawnId} started with sessionId=${sessionId}`);

          this.eventBus.broadcast('agent-online', {
            ...agentInfo,
            spawnId,
          });
        } else {
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

  private buildCommand(launchSpec: AgentLaunchSpec): string {
    if (launchSpec.kind === 'binary') {
      return launchSpec.binaryPath;
    }
    return 'cargo';
  }

  private buildArgs(config: SpawnConfig): string[] {
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
    args.push(...this.permissionArgs(config.permissionMode));
    if (config.appfsIdleWake) {
      args.push('--appfs-idle-wake');
    }
    if (config.sessionPath?.trim()) {
      args.push('--session', config.sessionPath.trim());
    }
    return args;
  }

  private buildEnvironment(config: SpawnConfig): NodeJS.ProcessEnv {
    const mountRoot = path.resolve(config.appfsMountRoot);
    return {
      ...process.env,
      ...config.env,
      APPFS_PRINCIPAL_ID: config.principalId,
      APPFS_MOUNT_ROOT: mountRoot,
      APPFS_RUNTIME_MANIFEST: path.join(mountRoot, '.well-known', 'appfs', 'runtime.json'),
    };
  }

  private permissionArgs(permissionMode: string): string[] {
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

  async shutdown(): Promise<void> {
    for (const [spawnId, managed] of this.agents.entries()) {
      console.log(`[ProcessManager] Shutting down agent ${managed.sessionId ?? spawnId}`);
      managed.process.stdin?.end();
      managed.process.kill('SIGTERM');
    }
    // Give processes time to exit gracefully
    await new Promise(resolve => setTimeout(resolve, 2000));
    for (const [, managed] of this.agents.entries()) {
      if (!managed.process.killed) {
        managed.process.kill('SIGKILL');
      }
    }
    this.agents.clear();
  }
}

function workspaceFingerprintFromSessionPath(sessionPath: string): string | undefined {
  if (!sessionPath) return undefined;
  const normalized = sessionPath.replace(/\\/g, '/');
  const parts = normalized.split('/').filter(Boolean);
  const index = parts.lastIndexOf('sessions');
  if (index >= 0 && parts.length > index + 1) {
    return parts[index + 1];
  }
  return undefined;
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

// ── Error types ──

export class AgentBusyError extends Error {
  constructor(sessionId: string) {
    super(`Agent ${sessionId} is currently busy with an active turn`);
    this.name = 'AgentBusyError';
  }
}
