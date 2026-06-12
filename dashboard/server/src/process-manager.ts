import { spawn, ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createInterface, Interface as ReadlineInterface } from 'node:readline';
import { EventBus } from './event-bus.js';
import type { AgentRegistry } from './agent-registry.js';
import type { ProjectRegistry } from './project-registry.js';
import type { AgentInfo, TokenUsage } from './types.js';
import { terminateChildProcessTree } from './child-process-utils.js';
import type { ModelConfigStore, ResolvedRuntimeModelConfig } from './model-config-store.js';
import { PersistentLog, resolveDashboardLogDir, safeLogFileSegment } from './persistent-log.js';

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
  projectId?: string;
  projectRoot?: string;
  modelProviderId?: string;
  modelId?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  runtimeModelConfigPath?: string;
  teamName?: string;
  taskListId?: string;
}

export interface DashboardControlEnv {
  apiOrigin: string;
  controlToken: string;
}

export function resolveProjectScopedSpawnConfig(
  spawnConfig: SpawnConfig,
  projectRegistry?: ProjectRegistry,
): SpawnConfig {
  const resolvedConfig: SpawnConfig = { ...spawnConfig };

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

export function buildManagedAppfsAttachId(principalId: string): string {
  const safePrincipalId = principalId
    .trim()
    .replace(/[^A-Za-z0-9_.-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 128);
  return `dashboard-${safePrincipalId || 'principal'}`;
}

export function samePrincipalScope(
  candidate: { principalId: string; projectId?: string },
  principalId: string,
  projectId?: string,
): boolean {
  return candidate.principalId === principalId
    && (!projectId || candidate.projectId === projectId);
}

export type PromptDelivery = 'prompt' | 'queue' | 'guidance';

export type PromptSubmissionStatus = 'accepted' | 'queued' | 'guidance';

export interface ProjectAgentResumeResult {
  resumed: Array<{ sessionId: string; spawnId: string }>;
  skipped: Array<{ sessionId: string; reason: string }>;
  errors: Array<{ sessionId: string; error: string }>;
}

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
  usage?: Partial<TokenUsage>;
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
  log?: PersistentLog;
}

type ManagedAgentMap = Map<string, ManagedAgent>;

interface PendingStartWaiter {
  resolve(result: { spawnId: string; sessionId: string }): void;
  reject(error: Error): void;
  timeout: NodeJS.Timeout;
}

const DEFAULT_AGENT_START_TIMEOUT_MS = 30_000;
const DASHBOARD_AGENT_LOG_RAW_STDOUT = 'DASHBOARD_AGENT_LOG_RAW_STDOUT';

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
  private pendingStartWaiters = new Map<string, PendingStartWaiter>();

  constructor(
    registry: AgentRegistry,
    private modelConfigStore?: ModelConfigStore,
    private dashboardControl?: DashboardControlEnv,
  ) {
    this.eventBus = EventBus.getInstance();
    this.registry = registry;
  }

  // ── Spawn ──

  spawn(spawnConfig: SpawnConfig): { spawnId: string } {
    const spawnId = `spawn-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

    const effectiveSpawnConfig = resolveProjectScopedSpawnConfig(
      spawnConfig,
      this.registry.projectRegistry,
    );
    this.assertSpawnConfig(effectiveSpawnConfig);
    const existing = this.findByPrincipalScope(
      effectiveSpawnConfig.principalId,
      effectiveSpawnConfig.projectId,
    );
    if (existing) {
      throw new SpawnConfigValidationError(
        `Managed agent already running for principal ${effectiveSpawnConfig.principalId}`
        + (effectiveSpawnConfig.projectId ? ` in project ${effectiveSpawnConfig.projectId}` : '')
        + '; stop it before starting or resuming another session.',
      );
    }
    this.resolveRuntimeModelConfig(effectiveSpawnConfig, spawnId);

    const args = this.buildArgs(effectiveSpawnConfig);
    const cmd = this.buildCommand(effectiveSpawnConfig.launchSpec);
    const agentLog = this.createAgentLog(spawnId, effectiveSpawnConfig);

    console.log(`[ProcessManager] Spawning agent ${spawnId}: ${cmd} ${args.join(' ')}`);
    agentLog?.appendLine(`[ProcessManager] spawnId=${spawnId}`);
    agentLog?.appendLine(`[ProcessManager] projectId=${effectiveSpawnConfig.projectId ?? '<none>'}`);
    agentLog?.appendLine(`[ProcessManager] principalId=${effectiveSpawnConfig.principalId}`);
    agentLog?.appendLine(`[ProcessManager] cwd=${effectiveSpawnConfig.cwd}`);
    agentLog?.appendLine(`[ProcessManager] appfsMountRoot=${effectiveSpawnConfig.appfsMountRoot}`);
    agentLog?.appendLine(`[ProcessManager] model=${effectiveSpawnConfig.model}`);
    agentLog?.appendLine(`[ProcessManager] sessionPath=${effectiveSpawnConfig.sessionPath ?? '<new>'}`);
    agentLog?.appendLine(`[ProcessManager] runtimeModelConfigPath=${effectiveSpawnConfig.runtimeModelConfigPath ?? '<none>'}`);
    agentLog?.appendLine(`[ProcessManager] cmd=${cmd} args=${args.join(' ')}`);
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

    const stdoutReader = createInterface({ input: childProcess.stdout! });
    const stderrReader = createInterface({ input: childProcess.stderr! });

    const managedAgent: ManagedAgent = {
      process: childProcess,
      sessionId: null,
      spawnConfig: effectiveSpawnConfig,
      status: 'starting',
      currentRequestId: null,
      controlEndpoint: null,
      stdoutReader,
      stderrReader,
      log: agentLog,
    };

    this.agents.set(spawnId, managedAgent);

    // ── stdout JSONL line parser ──
    stdoutReader.on('line', (line: string) => {
      this.handleStdoutLine(spawnId, line);
    });

    // ── stderr log forwarder ──
    stderrReader.on('line', (line: string) => {
      this.forwardStderrLine(spawnId, managedAgent, line);
    });

    // ── Process exit ──
    childProcess.on('exit', (code, signal) => {
      managedAgent.log?.appendLine(`[ProcessManager] exited code=${code} signal=${signal}`);
      this.handleManagedExit(spawnId, managedAgent, code, signal);
    });

    childProcess.on('error', (err) => {
      console.error(`[ProcessManager] Spawn error for ${spawnId}:`, err);
      managedAgent.log?.appendLine(`[ProcessManager] spawn error: ${err.stack ?? err.message}`);
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
      this.rejectStartWaiter(spawnId, err);
    });

    return { spawnId };
  }

  async spawnAndWaitStarted(
    spawnConfig: SpawnConfig,
    timeoutMs = agentStartTimeoutMs(),
  ): Promise<{ spawnId: string; sessionId: string }> {
    const { spawnId } = this.spawn(spawnConfig);
    const managed = this.agents.get(spawnId);
    if (managed?.sessionId) {
      return { spawnId, sessionId: managed.sessionId };
    }

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingStartWaiters.delete(spawnId);
        reject(new Error(`Agent ${spawnId} did not publish session_started within ${timeoutMs} ms`));
      }, timeoutMs);

      this.pendingStartWaiters.set(spawnId, {
        resolve,
        reject,
        timeout,
      });
    });
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

  async cancelTurn(sessionId: string, requestId?: string): Promise<{ requestId: string; status: 'cancelling' }> {
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

  stop(sessionId: string): boolean {
    const managed = this.findBySessionId(sessionId);
    if (!managed) return false;

    void terminateChildProcessTree(managed.process, {
      label: `agent ${managed.sessionId ?? sessionId}`,
      gracefulTimeoutMs: 5000,
    });

    return true;
  }

  findManagedAgentByPrincipal(
    principalId: string,
    projectId?: string,
  ): {
    pid?: number;
    sessionId: string | null;
    status: 'starting' | 'idle' | 'busy';
    principalId: string;
    model: string;
    permissionMode: string;
  } | null {
    const agent = this.findByPrincipalScope(principalId, projectId);
    if (agent) {
      return {
        pid: agent.process.pid,
        sessionId: agent.sessionId,
        status: agent.status,
        principalId: agent.spawnConfig.principalId,
        model: agent.spawnConfig.model,
        permissionMode: agent.spawnConfig.permissionMode,
      };
    }
    return null;
  }

  async stopPrincipal(principalId: string, projectId?: string): Promise<{ sessionId: string | null } | null> {
    const agent = this.findByPrincipalScope(principalId, projectId);
    if (agent) {
      const sessionId = agent.sessionId;
      await terminateChildProcessTree(agent.process, {
        label: `agent ${agent.sessionId ?? principalId}`,
        gracefulTimeoutMs: 5000,
      });
      return { sessionId };
    }
    return null;
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

  resumeProjectAgents(projectId: string): ProjectAgentResumeResult {
    const result: ProjectAgentResumeResult = {
      resumed: [],
      skipped: [],
      errors: [],
    };

    const project = this.registry.projectRegistry.getProject(projectId);
    if (!project) {
      result.errors.push({ sessionId: projectId, error: `Project ${projectId} not found` });
      return result;
    }

    const agents = latestResumableAgentPerPrincipal(
      this.registry.getAgents().filter(agent => agent.projectId === projectId),
    );

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
          modelProviderId: agent.modelProviderId,
          modelId: agent.modelId,
          contextWindowTokens: agent.contextWindowTokens,
          maxOutputTokens: agent.maxOutputTokens,
          runtimeModelConfigPath: agent.runtimeModelConfigPath,
          sessionPath: agent.sessionJsonlPath,
          projectId: project.projectId,
          projectRoot: project.projectRoot,
          cwd: project.projectRoot,
          appfsMountRoot: project.mountRoot,
        });
        result.resumed.push({ sessionId: agent.sessionId, spawnId });
      } catch (err: unknown) {
        result.errors.push({
          sessionId: agent.sessionId,
          error: err instanceof Error ? err.message : String(err),
        });
      }
    }

    return result;
  }

  getManagedAgents(): Array<{
    pid?: number;
    sessionId: string | null;
    status: 'starting' | 'idle' | 'busy';
    principalId: string;
    projectId?: string;
    model: string;
    permissionMode: string;
  }> {
    return Array.from(this.agents.values()).map(a => ({
      pid: a.process.pid,
      sessionId: a.sessionId,
      status: a.status,
      principalId: a.spawnConfig.principalId,
      projectId: a.spawnConfig.projectId,
      model: a.spawnConfig.model,
      permissionMode: a.spawnConfig.permissionMode,
    }));
  }

  getDefaultSpawnConfig(): SpawnConfig {
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

  private findBySessionId(sessionId: string): ManagedAgent | null {
    for (const agent of this.agents.values()) {
      if (agent.sessionId === sessionId) return agent;
    }
    return null;
  }

  private findBySessionPath(sessionPath: string): ManagedAgent | null {
    const target = path.resolve(sessionPath);
    for (const agent of this.agents.values()) {
      if (agent.spawnConfig.sessionPath && path.resolve(agent.spawnConfig.sessionPath) === target) {
        return agent;
      }
    }
    return null;
  }

  private findByPrincipalScope(principalId: string, projectId?: string): ManagedAgent | null {
    for (const agent of this.agents.values()) {
      if (samePrincipalScope(agent.spawnConfig, principalId, projectId)) {
        return agent;
      }
    }
    return null;
  }

  private assertSpawnConfig(config: SpawnConfig): void {
    const missing: string[] = [];
    if (!config.cwd?.trim()) missing.push('cwd');
    if (!config.principalId?.trim()) missing.push('principalId');
    if (!config.model?.trim()) missing.push('model');
    if (!config.appfsMountRoot?.trim()) missing.push('appfsMountRoot');
    if (!config.launchSpec) {
      missing.push('launchSpec');
    } else if (config.launchSpec.kind === 'binary') {
      if (!config.launchSpec.binaryPath?.trim()) missing.push('launchSpec.binaryPath');
    } else if (config.launchSpec.kind === 'cargo') {
      if (!config.launchSpec.manifestPath?.trim()) missing.push('launchSpec.manifestPath');
      if (!config.launchSpec.package?.trim()) missing.push('launchSpec.package');
    }

    if (missing.length > 0) {
      throw new SpawnConfigValidationError(`Missing required fields: ${missing.join(', ')}`);
    }
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
      this.appendStdoutLog(managed, line);
      this.eventBus.broadcast('process-log', {
        agentId: managed.sessionId ?? spawnId,
        spawnId,
        stream: 'stdout',
        text: line,
      });
      return;
    }

    const agentId = managed.sessionId ?? spawnId;
    this.appendHeadlessEventLog(managed, event);

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
            currentContextTokens: 0,
            projectId: managed.spawnConfig.projectId,
            projectRoot: managed.spawnConfig.projectRoot,
            modelProviderId: managed.spawnConfig.modelProviderId,
            modelId: managed.spawnConfig.modelId,
            contextWindowTokens: managed.spawnConfig.contextWindowTokens,
            maxOutputTokens: managed.spawnConfig.maxOutputTokens,
            runtimeModelConfigPath: managed.spawnConfig.runtimeModelConfigPath,
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
          this.resolveStartWaiter(spawnId, sessionId);
        } else {
          this.eventBus.broadcast('agent-online', {
            sessionId: spawnId,
            spawnId,
            controlMode: 'managed',
          });
          this.rejectStartWaiter(
            spawnId,
            new Error(`Agent ${spawnId} emitted session_started without session_id`),
          );
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

  private forwardStderrLine(spawnId: string, managedAgent: ManagedAgent, line: string): void {
    const agentId = managedAgent.sessionId ?? spawnId;
    managedAgent.log?.appendLine(`[stderr] ${line}`);
    this.eventBus.broadcast('process-log', {
      agentId,
      spawnId,
      stream: 'stderr',
      text: line,
    });
  }

  private appendStdoutLog(managedAgent: ManagedAgent, line: string): void {
    if (shouldLogRawAgentStdout()) {
      managedAgent.log?.appendLine(`[stdout] ${line}`);
    } else {
      managedAgent.log?.appendLine(`[stdout] <non-json ${line.length} chars; set ${DASHBOARD_AGENT_LOG_RAW_STDOUT}=1 to persist raw stdout>`);
    }
  }

  private appendHeadlessEventLog(managedAgent: ManagedAgent, event: HeadlessEvent): void {
    if (shouldLogRawAgentStdout()) {
      managedAgent.log?.appendLine(`[stdout] ${JSON.stringify(event)}`);
      return;
    }

    const parts = [`type=${event.type}`];
    appendLogPart(parts, 'session', event.session_id);
    appendLogPart(parts, 'principal', event.principal_id);
    appendLogPart(parts, 'request', event.request_id);
    appendLogPart(parts, 'turn', event.turn_id);
    appendLogPart(parts, 'tool', event.tool_name);
    appendLogPart(parts, 'status', event.status);
    if (event.message) {
      appendLogPart(parts, 'message', truncateForLog(event.message, 240));
    }
    if (event.type === 'assistant_delta') {
      appendLogPart(parts, 'text_len', String(event.text?.length ?? 0));
    }
    managedAgent.log?.appendLine(`[stdout-event] ${parts.join(' ')}`);
  }

  private handleManagedExit(
    spawnId: string,
    managedAgent: ManagedAgent,
    code: number | null,
    signal: NodeJS.Signals | null,
  ): void {
    const agentId = managedAgent.sessionId ?? spawnId;
    console.log(`[ProcessManager] Agent ${agentId} exited with code=${code}, signal=${signal}`);

    this.eventBus.broadcast('agent-offline', {
      sessionId: agentId,
      spawnId,
      principalId: managedAgent.spawnConfig.principalId,
      projectId: managedAgent.spawnConfig.projectId,
      code,
      signal,
    });

    if (managedAgent.sessionId) {
      const existingAgent = this.registry.getAgent(managedAgent.sessionId);
      if (existingAgent) {
        this.registry.registerAgent({ ...existingAgent, status: 'offline' });
      }
      if (managedAgent.spawnConfig.projectId) {
        this.registry.projectRegistry.detachAgent(
          managedAgent.spawnConfig.projectId,
          managedAgent.sessionId,
        );
      }
    }

    managedAgent.stdoutReader.close();
    managedAgent.stderrReader.close();
    managedAgent.process.stdout?.destroy();
    managedAgent.process.stderr?.destroy();
    managedAgent.process.stdin?.end();
    managedAgent.process.stdin?.destroy();
    this.agents.delete(spawnId);
    if (!managedAgent.sessionId) {
      this.rejectStartWaiter(
        spawnId,
        new Error(`Agent ${spawnId} exited before session_started (code=${code}, signal=${signal})`),
      );
    }
    if (managedAgent.sessionId) {
      this.pendingSpawnMap.delete(managedAgent.sessionId);
    }
  }

  private resolveStartWaiter(spawnId: string, sessionId: string): void {
    const waiter = this.pendingStartWaiters.get(spawnId);
    if (!waiter) {
      return;
    }
    clearTimeout(waiter.timeout);
    this.pendingStartWaiters.delete(spawnId);
    waiter.resolve({ spawnId, sessionId });
  }

  private rejectStartWaiter(spawnId: string, error: Error): void {
    const waiter = this.pendingStartWaiters.get(spawnId);
    if (!waiter) {
      return;
    }
    clearTimeout(waiter.timeout);
    this.pendingStartWaiters.delete(spawnId);
    waiter.reject(error);
  }

  private createAgentLog(spawnId: string, config: SpawnConfig): PersistentLog | undefined {
    try {
      const projectSegment = safeLogFileSegment(config.projectId ?? config.projectRoot ?? 'project');
      const principalSegment = safeLogFileSegment(config.principalId);
      const spawnSegment = safeLogFileSegment(spawnId);
      return new PersistentLog(path.join(
        resolveDashboardLogDir(),
        'agents',
        `agent-${projectSegment}-${principalSegment}-${spawnSegment}.log`,
      ));
    } catch (err) {
      console.warn(
        `[ProcessManager] Failed to create persistent agent log for ${spawnId}:`,
        err,
      );
      return undefined;
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

  private buildEnvironment(config: SpawnConfig): NodeJS.ProcessEnv {
    const mountRoot = path.resolve(config.appfsMountRoot);
    const dashboardEnv: Record<string, string> = {};
    if (this.dashboardControl && config.projectId) {
      dashboardEnv.APPFS_DASHBOARD_API_ORIGIN = this.dashboardControl.apiOrigin;
      dashboardEnv.APPFS_DASHBOARD_PROJECT_ID = config.projectId;
      dashboardEnv.APPFS_DASHBOARD_CONTROL_TOKEN = this.dashboardControl.controlToken;
    }
    const taskScopeEnv: Record<string, string> = {};
    if (config.taskListId?.trim()) {
      const taskListId = config.taskListId.trim();
      taskScopeEnv.APPFS_TASK_LIST_ID = taskListId;
      taskScopeEnv.CLAW_TASK_LIST_ID = taskListId;
      taskScopeEnv.CLAUDE_CODE_TASK_LIST_ID = taskListId;
    }
    if (config.teamName?.trim()) {
      const teamName = config.teamName.trim();
      taskScopeEnv.APPFS_TEAM_NAME = teamName;
      taskScopeEnv.CLAUDE_CODE_TEAM_NAME = teamName;
    }
    return {
      ...process.env,
      ...config.env,
      ...dashboardEnv,
      ...taskScopeEnv,
      APPFS_PRINCIPAL_ID: config.principalId,
      APPFS_ATTACH_ID: buildManagedAppfsAttachId(config.principalId),
      APPFS_MOUNT_ROOT: mountRoot,
      APPFS_RUNTIME_MANIFEST: path.join(mountRoot, '.well-known', 'appfs', 'runtime.json'),
    };
  }

  private resolveRuntimeModelConfig(config: SpawnConfig, spawnId: string): void {
    if (config.runtimeModelConfigPath && this.hydrateExistingRuntimeModelConfig(config)) {
      return;
    }

    if (!this.modelConfigStore) {
      return;
    }

    // Fallback: If no providerId is specified, try to find a matching provider for the model name in the catalog
    if (!config.modelProviderId && config.model && config.model !== 'unknown') {
      try {
        const allProviders = this.modelConfigStore.load().providers;
        const matches: Array<{ providerId: string; modelId: string }> = [];
        for (const p of allProviders) {
          for (const m of p.models) {
            if (m.name === config.model || m.id === config.model) {
              matches.push({ providerId: p.id, modelId: m.id });
            }
          }
        }
        if (matches.length === 1) {
          config.modelProviderId = matches[0].providerId;
          config.modelId = matches[0].modelId;
          console.log(`[ProcessManager] Fallback matched model "${config.model}" to provider "${config.modelProviderId}" and modelId "${config.modelId}"`);
        } else if (matches.length > 1) {
          console.warn(`[ProcessManager] Ambiguous model name "${config.model}" matches multiple providers: ${matches.map(m => m.providerId).join(', ')}. Skipping auto-fill.`);
        }
      } catch (err) {
        console.error(`[ProcessManager] Error running model provider fallback:`, err);
      }
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

  private hydrateExistingRuntimeModelConfig(config: SpawnConfig): boolean {
    const configPath = config.runtimeModelConfigPath?.trim();
    if (!configPath) {
      return false;
    }

    if (!fs.existsSync(configPath)) {
      console.warn(`[ProcessManager] Runtime model config not found at ${configPath}; resolving from model catalog instead.`);
      config.runtimeModelConfigPath = undefined;
      return false;
    }

    try {
      const resolved = JSON.parse(fs.readFileSync(configPath, 'utf8')) as Partial<ResolvedRuntimeModelConfig>;
      if (!resolved.providerId || !resolved.modelId || !resolved.model?.name) {
        console.warn(`[ProcessManager] Runtime model config at ${configPath} is incomplete; resolving from model catalog instead.`);
        config.runtimeModelConfigPath = undefined;
        return false;
      }
      config.modelProviderId ??= resolved.providerId;
      config.modelId ??= resolved.modelId;
      config.contextWindowTokens ??= resolved.model?.contextWindowTokens;
      config.maxOutputTokens ??= resolved.model?.maxOutputTokens;
      if (!config.model?.trim() && resolved.model?.name) {
        config.model = resolved.model.name;
      }
      return true;
    } catch (err) {
      console.warn(
        `[ProcessManager] Failed to read runtime model config at ${configPath}; resolving from model catalog instead:`,
        err,
      );
      config.runtimeModelConfigPath = undefined;
      return false;
    }
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
    for (const spawnId of Array.from(this.pendingStartWaiters.keys())) {
      this.rejectStartWaiter(spawnId, new Error(`Agent ${spawnId} was shut down before session_started`));
    }
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

function agentStartTimeoutMs(): number {
  const parsed = Number.parseInt(process.env.DASHBOARD_AGENT_START_TIMEOUT_MS ?? '', 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_AGENT_START_TIMEOUT_MS;
}

function shouldLogRawAgentStdout(): boolean {
  const value = process.env[DASHBOARD_AGENT_LOG_RAW_STDOUT];
  return value === '1' || value?.toLowerCase() === 'true';
}

function appendLogPart(parts: string[], key: string, value: string | undefined): void {
  if (value === undefined || value === '') {
    return;
  }
  parts.push(`${key}=${value}`);
}

function truncateForLog(value: string, maxLength: number): string {
  if (value.length <= maxLength) {
    return value;
  }
  return `${value.slice(0, maxLength)}...`;
}

export function latestResumableAgentPerPrincipal(agents: AgentInfo[]): AgentInfo[] {
  const latestByPrincipal = new Map<string, AgentInfo>();
  for (const agent of agents) {
    if (agent.archived) {
      continue;
    }
    const principalId = agent.principalId || agent.name;
    if (!principalId) {
      continue;
    }
    const current = latestByPrincipal.get(principalId);
    if (!current || agent.startedAt > current.startedAt) {
      latestByPrincipal.set(principalId, agent);
    }
  }
  return Array.from(latestByPrincipal.values())
    .sort((a, b) => b.startedAt - a.startedAt);
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

export class AgentNoActiveTurnError extends Error {
  constructor(sessionId: string) {
    super(`Agent ${sessionId} does not have an active turn to cancel`);
    this.name = 'AgentNoActiveTurnError';
  }
}

export class SpawnConfigValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SpawnConfigValidationError';
  }
}
