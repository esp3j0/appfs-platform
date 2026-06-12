import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import type { AgentRegistry } from './agent-registry.js';
import type { AgentProcessManager, ProjectAgentResumeResult, SpawnConfig } from './process-manager.js';
import { latestResumableAgentPerPrincipal, samePrincipalScope } from './process-manager.js';
import type { ProjectRecord, ProjectRegistry } from './project-registry.js';
import type { AgentInfo } from './types.js';

const SAFE_PRINCIPAL_ID = /^[A-Za-z0-9_.-]{1,128}$/;
const ATTACH_STALE_AFTER_MS = 90_000;
const APPFS_ACTION_WAIT_TIMEOUT_MS = Number.parseInt(
  process.env.DASHBOARD_APPFS_ACTION_WAIT_TIMEOUT_MS ?? '',
  10,
) || 10_000;
const APPFS_ACTION_POLL_MS = 100;
const DEFAULT_BOOTSTRAP_RESUME_CONCURRENCY = 2;
const MAX_BOOTSTRAP_RESUME_CONCURRENCY = 8;

export interface PrincipalRegistryPrincipal {
  principal_id: string;
  display_name?: string;
  description?: string | null;
  kind?: string;
  created_at?: string;
  updated_at?: string;
  presence?: string;
  active_attach_count?: number;
  active_attaches?: Array<{ attach_id?: string; last_seen_at?: string; [key: string]: unknown }>;
  agent_status?: Record<string, unknown> | null;
}

export interface PrincipalLifecycleInfo extends PrincipalRegistryPrincipal {
  online: boolean;
  status: string;
  pid?: number;
  sessionId: string | null;
  model?: string;
  permissionMode?: string;
}

export interface PrincipalRegistryDoc {
  version: number;
  default_principal_id?: string;
  principals: PrincipalRegistryPrincipal[];
}

export interface PrincipalLifecycleListResponse {
  version: number;
  default_principal_id?: string;
  principals: PrincipalLifecycleInfo[];
}

export interface PrincipalCreateRequest {
  principalId: string;
  displayName?: string;
  description?: string | null;
  kind?: string;
}

export interface PrincipalStartRequest {
  model?: string;
  modelProviderId?: string;
  modelId?: string;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  permissionMode?: string;
  teamName?: string;
  taskListId?: string;
}

export interface PrincipalResumeRequest extends PrincipalStartRequest {
  sessionId?: string;
}

interface ManagedPrincipalInfo {
  pid?: number;
  sessionId: string | null;
  status: 'starting' | 'idle' | 'busy';
  principalId: string;
  projectId?: string;
  model: string;
  permissionMode: string;
}

interface PrincipalLifecycleDeps {
  projectRegistry: Pick<ProjectRegistry, 'getProject'>;
  agentRegistry: Pick<AgentRegistry, 'archiveSessionsForPrincipal' | 'discoverProject' | 'getAgents'>;
  processManager: Pick<
    AgentProcessManager,
    | 'findManagedAgentByPrincipal'
    | 'getDefaultSpawnConfig'
    | 'getManagedAgents'
    | 'spawn'
    | 'spawnAndWaitStarted'
    | 'stopPrincipal'
  >;
}

interface PrincipalStatusEntry extends Record<string, unknown> {
  principal_id?: string;
}

interface PrincipalStatusDoc {
  principals?: PrincipalStatusEntry[];
}

interface AppfsControlEvent {
  path?: string;
  type?: string;
  client_token?: string;
  content?: {
    principal_event?: string;
    principal_id?: string;
    [key: string]: unknown;
  };
  error?: {
    code?: string;
    message?: string;
    [key: string]: unknown;
  };
}

type BootstrapResumeOutcome =
  | { kind: 'resumed'; sessionId: string; spawnId: string }
  | { kind: 'skipped'; sessionId: string; reason: string }
  | { kind: 'error'; sessionId: string; error: string };

export class PrincipalLifecycleError extends Error {
  constructor(
    public statusCode: number,
    message: string,
    public code?: string,
  ) {
    super(message);
    this.name = 'PrincipalLifecycleError';
  }
}

export function normalizePrincipalId(input: string): string {
  const principalId = input.trim();
  if (!SAFE_PRINCIPAL_ID.test(principalId) || principalId === '.' || principalId === '..') {
    throw new PrincipalLifecycleError(400, `Invalid principalId: ${input}`);
  }
  return principalId;
}

export function principalControlDir(mountRoot: string): string {
  return path.join(mountRoot, '_appfs', 'principals');
}

export function readPrincipalViews(mountRoot: string): PrincipalRegistryDoc {
  const registryPath = path.join(mountRoot, '_appfs', 'principals.registry.json');
  const statusPath = path.join(principalControlDir(mountRoot), 'status.res.json');
  const doc: PrincipalRegistryDoc = fs.existsSync(registryPath)
    ? JSON.parse(fs.readFileSync(registryPath, 'utf8'))
    : { version: 1, principals: [] };
  const statusDoc: PrincipalStatusDoc | null = fs.existsSync(statusPath)
    ? JSON.parse(fs.readFileSync(statusPath, 'utf8'))
    : null;
  const statusByPrincipal = new Map<string, PrincipalStatusEntry>();

  for (const entry of statusDoc?.principals ?? []) {
    if (typeof entry?.principal_id === 'string') {
      statusByPrincipal.set(entry.principal_id, entry);
    }
  }

  return {
    ...doc,
    principals: doc.principals.map((principal) => ({
      ...principal,
      agent_status: principal.agent_status ?? statusByPrincipal.get(principal.principal_id) ?? null,
    })),
  };
}

export function appendPrincipalAction(
  mountRoot: string,
  actionFile: 'create_principal.act' | 'update_principal.act' | 'delete_principal.act' | 'detach_principal.act',
  payload: Record<string, unknown>,
): string {
  const dir = principalControlDir(mountRoot);
  fs.mkdirSync(dir, { recursive: true });
  const clientToken =
    typeof payload.client_token === 'string'
      ? payload.client_token
      : `dashboard-${Date.now()}-${crypto.randomUUID()}`;
  const line = `${JSON.stringify({ ...payload, client_token: clientToken })}\n`;
  fs.appendFileSync(path.join(dir, actionFile), line, 'utf8');
  return clientToken;
}

export class PrincipalLifecycleService {
  constructor(private deps: PrincipalLifecycleDeps) {}

  listPrincipals(projectId: string): PrincipalLifecycleListResponse {
    const project = this.requireProject(projectId);
    this.deps.agentRegistry.discoverProject(project.projectRoot);
    const doc = readPrincipalViews(project.mountRoot);
    const managedAgents = this.deps.processManager.getManagedAgents();
    const registryAgents = this.deps.agentRegistry.getAgents()
      .filter((agent) => agent.projectId === project.projectId);

    return {
      ...doc,
      principals: doc.principals.map((principal) => {
        const managed = managedAgents.find((agent) => (
          samePrincipalScope(agent, principal.principal_id, project.projectId)
        ));
        const discovered = latestAgentForPrincipal(registryAgents, principal.principal_id);
        const active = managed ?? discovered;

        return {
          ...principal,
          online: active ? active.status !== 'offline' : false,
          status: active?.status ?? 'offline',
          pid: active?.pid && active.pid !== 0 ? active.pid : undefined,
          sessionId: active?.sessionId ?? null,
          model: active?.model,
          permissionMode: isManagedPrincipalInfo(active) ? active.permissionMode : undefined,
        };
      }),
    };
  }

  async createPrincipal(projectId: string, input: PrincipalCreateRequest) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(input.principalId);
    appendPrincipalAction(project.mountRoot, 'create_principal.act', {
      principal_id: principalId,
      display_name: input.displayName ?? principalId,
      description: input.description,
      kind: input.kind ?? 'agent',
    });
    return { status: 'created' as const, principal: { principal_id: principalId } };
  }

  async ensurePrincipalReady(projectId: string, input: PrincipalCreateRequest) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(input.principalId);
    const clientToken = appendPrincipalAction(project.mountRoot, 'create_principal.act', {
      principal_id: principalId,
      display_name: input.displayName ?? principalId,
      description: input.description,
      kind: input.kind ?? 'agent',
    });
    const event = await waitForPrincipalCreateAction(project.mountRoot, clientToken, principalId);
    return {
      status: event.content?.principal_event === 'principal.exists' ? 'exists' as const : 'created' as const,
      principal: { principal_id: principalId },
      appInstances: event.content?.app_instances,
    };
  }

  async deletePrincipal(projectId: string, principalIdInput: string) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(principalIdInput);

    const managed = this.deps.processManager.findManagedAgentByPrincipal(
      principalId,
      project.projectId,
    );
    if (managed) {
      throw new PrincipalLifecycleError(409, `Principal ${principalId} has a running managed agent`);
    }

    const activeRegistryAgent = this.deps.agentRegistry.getAgents().find((agent) => (
      !agent.archived && samePrincipalScope(agent, principalId, project.projectId) && agent.status === 'online'
    ));
    if (activeRegistryAgent) {
      throw new PrincipalLifecycleError(409, `Principal ${principalId} has an online agent`);
    }

    const principal = readPrincipalViews(project.mountRoot).principals
      .find((item) => item.principal_id === principalId);
    const attachState = principal ? principalAttachState(principal) : { active: false, hasStaleAttaches: false };
    if (attachState.active) {
      throw new PrincipalLifecycleError(409, `Principal ${principalId} has an active AppFS attach`);
    }

    // Best-effort detach stale attaches before deleting, so we don't rely
    // on the AppFS sweep timer.  Fire and forget — the delete with
    // force=true will clean up anything that remains.
    if (attachState.hasStaleAttaches && principal) {
      const attach = freshestAttach(principal);
      if (attach?.attach_id) {
        appendPrincipalAction(project.mountRoot, 'detach_principal.act', {
          principal_id: principalId,
          attach_id: attach.attach_id,
          reason: 'pre_delete_stale_cleanup',
        });
      }
    }

    const clientToken = appendPrincipalAction(project.mountRoot, 'delete_principal.act', {
      principal_id: principalId,
      ...(attachState.hasStaleAttaches ? { force: true } : {}),
    });
    await waitForPrincipalDeleteAction(project.mountRoot, clientToken, principalId);
    const archivedSessions = this.deps.agentRegistry.archiveSessionsForPrincipal(
      principalId,
      project.projectId,
      'principal_deleted',
    );
    return {
      status: 'deleted' as const,
      principalId,
      archivedSessions: archivedSessions.map((agent) => agent.sessionId),
    };
  }

  async resumeProjectPrincipals(projectId: string): Promise<ProjectAgentResumeResult> {
    const project = this.requireProject(projectId);
    const result: ProjectAgentResumeResult = {
      resumed: [],
      skipped: [],
      errors: [],
    };

    this.deps.agentRegistry.discoverProject(project.projectRoot);
    const defaultPrincipalId = readDefaultPrincipalId(project.mountRoot, project.projectId);
    const agents = orderBootstrapResumeAgents(
      latestResumableAgentPerPrincipal(
        this.deps.agentRegistry.getAgents().filter((agent) => agent.projectId === project.projectId),
      ),
      defaultPrincipalId,
    );
    const concurrency = bootstrapResumeConcurrency();

    console.log(
      `[PrincipalLifecycle] Bootstrap resume project=${project.projectId} candidates=${agents.length} `
      + `concurrency=${concurrency}${defaultPrincipalId ? ` default=${defaultPrincipalId}` : ''}`,
    );

    const skippedOutcome = (agent: AgentInfo, reason: string): BootstrapResumeOutcome => {
      console.log(
        `[PrincipalLifecycle] Bootstrap resume skipped project=${project.projectId} `
        + `principal=${agent.principalId || agent.name || '<missing>'} session=${agent.sessionId} reason=${reason}`,
      );
      return { kind: 'skipped', sessionId: agent.sessionId, reason };
    };

    const outcomes = await mapWithConcurrency(agents, concurrency, async (agent): Promise<BootstrapResumeOutcome> => {
      if (!agent.sessionJsonlPath) {
        return skippedOutcome(agent, 'missing session path');
      }
      if (agent.status === 'online') {
        return skippedOutcome(agent, 'already online');
      }

      const principalId = agent.principalId || agent.name;
      if (!principalId) {
        return skippedOutcome(agent, 'missing principal id');
      }

      try {
        console.log(
          `[PrincipalLifecycle] Bootstrap resume starting project=${project.projectId} `
          + `principal=${principalId} session=${agent.sessionId}`,
        );
        const resume = await this.resumePrincipalForBootstrap(project, principalId, {
          sessionId: agent.sessionId,
        });
        if ('spawnId' in resume && typeof resume.spawnId === 'string') {
          console.log(
            `[PrincipalLifecycle] Bootstrap resume started project=${project.projectId} `
            + `principal=${principalId} session=${agent.sessionId} spawn=${resume.spawnId}`,
          );
          return { kind: 'resumed', sessionId: agent.sessionId, spawnId: resume.spawnId };
        }
        return skippedOutcome(agent, resume.status);
      } catch (err: unknown) {
        const error = err instanceof Error ? err.message : String(err);
        console.warn(
          `[PrincipalLifecycle] Bootstrap resume failed project=${project.projectId} `
          + `principal=${principalId} session=${agent.sessionId}: ${error}`,
        );
        return {
          kind: 'error',
          sessionId: agent.sessionId,
          error,
        };
      }
    });

    for (const outcome of outcomes) {
      if (outcome.kind === 'resumed') {
        result.resumed.push({ sessionId: outcome.sessionId, spawnId: outcome.spawnId });
      } else if (outcome.kind === 'skipped') {
        result.skipped.push({ sessionId: outcome.sessionId, reason: outcome.reason });
      } else {
        result.errors.push({ sessionId: outcome.sessionId, error: outcome.error });
      }
    }

    console.log(
      `[PrincipalLifecycle] Bootstrap resume complete project=${project.projectId} `
      + `resumed=${result.resumed.length} skipped=${result.skipped.length} errors=${result.errors.length}`,
    );

    return result;
  }

  async startPrincipal(
    projectId: string,
    principalIdInput: string,
    input: PrincipalStartRequest = {},
  ) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(principalIdInput);
    const existing = this.deps.processManager.findManagedAgentByPrincipal(
      principalId,
      project.projectId,
    );
    if (existing) {
      return {
        status: 'already-running' as const,
        principalId,
        sessionId: existing.sessionId,
      };
    }

    const config = this.buildSpawnConfig(project, principalId, input);
    const { spawnId } = this.deps.processManager.spawn(config);
    return { status: 'spawning' as const, spawnId, principalId };
  }

  async startPrincipalAndWait(
    projectId: string,
    principalIdInput: string,
    input: PrincipalStartRequest = {},
  ) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(principalIdInput);
    const existing = this.deps.processManager.findManagedAgentByPrincipal(
      principalId,
      project.projectId,
    );
    if (existing) {
      if (!existing.sessionId) {
        throw new PrincipalLifecycleError(
          409,
          `Principal ${principalId} already has a managed agent that is still starting`,
          'AGENT_STARTING',
        );
      }
      return {
        status: 'already-running' as const,
        principalId,
        sessionId: existing.sessionId,
        model: existing.model,
      };
    }

    const config = this.buildSpawnConfig(project, principalId, input);
    let started: { spawnId: string; sessionId: string };
    try {
      started = await this.deps.processManager.spawnAndWaitStarted(config);
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      if (message.includes('did not publish session_started')) {
        throw new PrincipalLifecycleError(504, message, 'AGENT_START_TIMEOUT');
      }
      throw err;
    }
    return {
      status: 'started' as const,
      spawnId: started.spawnId,
      principalId,
      sessionId: started.sessionId,
      model: config.model,
    };
  }

  async stopPrincipal(projectId: string, principalIdInput: string) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(principalIdInput);
    const stopped = await this.deps.processManager.stopPrincipal(principalId, project.projectId);
    if (!stopped) {
      throw new PrincipalLifecycleError(404, `No managed agent found for principal ${principalId}`);
    }

    const detach = await detachPrincipalIfAttached(project.mountRoot, principalId);
    return {
      status: detach.detached ? 'stopped' as const : 'stopping' as const,
      principalId,
      sessionId: stopped.sessionId,
      detach,
    };
  }

  async resumePrincipal(
    projectId: string,
    principalIdInput: string,
    input: PrincipalResumeRequest = {},
  ) {
    const project = this.requireProject(projectId);
    const principalId = normalizePrincipalId(principalIdInput);
    const existing = this.deps.processManager.findManagedAgentByPrincipal(
      principalId,
      project.projectId,
    );
    if (existing) {
      return {
        status: 'already-running' as const,
        principalId,
        sessionId: existing.sessionId,
      };
    }

    const session = this.findResumeSession(project, principalId, input.sessionId);
    if (!session.sessionJsonlPath) {
      throw new PrincipalLifecycleError(404, `No resumable session found for principal ${principalId}`);
    }

    const config = this.buildSpawnConfig(project, principalId, {
      ...input,
      model: input.model ?? session.model,
      modelProviderId: input.modelProviderId ?? session.modelProviderId,
      modelId: input.modelId ?? session.modelId,
      contextWindowTokens: input.contextWindowTokens ?? session.contextWindowTokens,
      maxOutputTokens: input.maxOutputTokens ?? session.maxOutputTokens,
    });
    config.sessionPath = session.sessionJsonlPath;
    config.runtimeModelConfigPath = session.runtimeModelConfigPath;

    const { spawnId } = this.deps.processManager.spawn(config);
    return {
      status: 'spawning' as const,
      spawnId,
      principalId,
      sessionId: session.sessionId,
    };
  }

  private async resumePrincipalForBootstrap(
    project: ProjectRecord,
    principalIdInput: string,
    input: PrincipalResumeRequest = {},
  ) {
    const principalId = normalizePrincipalId(principalIdInput);
    const existing = this.deps.processManager.findManagedAgentByPrincipal(
      principalId,
      project.projectId,
    );
    if (existing) {
      return {
        status: 'already-running' as const,
        principalId,
        sessionId: existing.sessionId,
      };
    }

    const session = this.findResumeSession(project, principalId, input.sessionId);
    if (!session.sessionJsonlPath) {
      throw new PrincipalLifecycleError(404, `No resumable session found for principal ${principalId}`);
    }

    const config = this.buildSpawnConfig(project, principalId, {
      ...input,
      model: input.model ?? session.model,
      modelProviderId: input.modelProviderId ?? session.modelProviderId,
      modelId: input.modelId ?? session.modelId,
      contextWindowTokens: input.contextWindowTokens ?? session.contextWindowTokens,
      maxOutputTokens: input.maxOutputTokens ?? session.maxOutputTokens,
    });
    config.sessionPath = session.sessionJsonlPath;
    config.runtimeModelConfigPath = session.runtimeModelConfigPath;

    const { spawnId } = await this.deps.processManager.spawnAndWaitStarted(config);
    return {
      status: 'spawning' as const,
      spawnId,
      principalId,
      sessionId: session.sessionId,
    };
  }

  private requireProject(projectId: string): ProjectRecord {
    const project = this.deps.projectRegistry.getProject(projectId);
    if (!project) {
      throw new PrincipalLifecycleError(404, `Project ${projectId} not found`);
    }
    return project;
  }

  private buildSpawnConfig(
    project: ProjectRecord,
    principalId: string,
    input: PrincipalStartRequest,
  ): SpawnConfig {
    const base = this.deps.processManager.getDefaultSpawnConfig();
    return {
      ...base,
      principalId,
      projectId: project.projectId,
      projectRoot: project.projectRoot,
      cwd: project.projectRoot,
      appfsMountRoot: project.mountRoot,
      model: input.model ?? base.model,
      modelProviderId: input.modelProviderId,
      modelId: input.modelId,
      contextWindowTokens: input.contextWindowTokens,
      maxOutputTokens: input.maxOutputTokens,
      permissionMode: input.permissionMode ?? base.permissionMode,
      teamName: input.teamName,
      taskListId: input.taskListId,
    };
  }

  private findResumeSession(
    project: ProjectRecord,
    principalId: string,
    sessionId?: string,
  ): AgentInfo {
    this.deps.agentRegistry.discoverProject(project.projectRoot);

    const candidates = this.deps.agentRegistry.getAgents()
      .filter((agent) => !agent.archived && samePrincipalScope(agent, principalId, project.projectId));

    if (sessionId) {
      const exact = candidates.find((agent) => agent.sessionId === sessionId);
      if (!exact) {
        throw new PrincipalLifecycleError(
          404,
          `No resumable session ${sessionId} found for principal ${principalId}`,
        );
      }
      return exact;
    }

    const latest = candidates
      .filter((agent) => Boolean(agent.sessionJsonlPath))
      .sort((a, b) => b.startedAt - a.startedAt)[0];

    if (!latest) {
      throw new PrincipalLifecycleError(404, `No resumable session found for principal ${principalId}`);
    }

    return latest;
  }
}

function latestAgentForPrincipal(agents: AgentInfo[], principalId: string): AgentInfo | undefined {
  return agents
    .filter((agent) => !agent.archived && agent.principalId === principalId)
    .sort((a, b) => {
      if (a.status === 'online' && b.status !== 'online') return -1;
      if (a.status !== 'online' && b.status === 'online') return 1;
      return b.startedAt - a.startedAt;
    })[0];
}

function bootstrapResumeConcurrency(): number {
  const parsed = Number.parseInt(process.env.DASHBOARD_BOOTSTRAP_RESUME_CONCURRENCY ?? '', 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return DEFAULT_BOOTSTRAP_RESUME_CONCURRENCY;
  }
  return Math.min(parsed, MAX_BOOTSTRAP_RESUME_CONCURRENCY);
}

function orderBootstrapResumeAgents(agents: AgentInfo[], defaultPrincipalId?: string): AgentInfo[] {
  if (!defaultPrincipalId) {
    return agents;
  }
  return [...agents].sort((left, right) => {
    const leftPrincipal = left.principalId || left.name;
    const rightPrincipal = right.principalId || right.name;
    const leftDefault = leftPrincipal === defaultPrincipalId;
    const rightDefault = rightPrincipal === defaultPrincipalId;
    if (leftDefault === rightDefault) {
      return 0;
    }
    return leftDefault ? -1 : 1;
  });
}

function readDefaultPrincipalId(mountRoot: string, projectId: string): string | undefined {
  try {
    const defaultPrincipalId = readPrincipalViews(mountRoot).default_principal_id;
    return typeof defaultPrincipalId === 'string' && defaultPrincipalId.trim()
      ? defaultPrincipalId
      : undefined;
  } catch (err: unknown) {
    console.warn(
      `[PrincipalLifecycle] Could not read default principal for bootstrap resume project=${projectId}: `
      + `${err instanceof Error ? err.message : String(err)}`,
    );
    return undefined;
  }
}

async function mapWithConcurrency<T, R>(
  items: T[],
  concurrency: number,
  worker: (item: T, index: number) => Promise<R>,
): Promise<R[]> {
  const results: R[] = new Array(items.length);
  let nextIndex = 0;
  const workerCount = Math.min(Math.max(1, concurrency), items.length);

  await Promise.all(Array.from({ length: workerCount }, async () => {
    while (true) {
      const index = nextIndex;
      nextIndex += 1;
      if (index >= items.length) {
        return;
      }
      results[index] = await worker(items[index], index);
    }
  }));

  return results;
}

async function waitForPrincipalDeleteAction(
  mountRoot: string,
  clientToken: string,
  principalId: string,
): Promise<void> {
  await waitForPrincipalControlAction({
    mountRoot,
    clientToken,
    principalId,
    path: '/_appfs/principals/delete_principal.act',
    completedEvent: 'principal.deleted',
    timeoutMessage: `Timed out waiting for AppFS to delete principal ${principalId}`,
    failureMessage: `AppFS delete_principal failed for ${principalId}`,
  });
}

async function waitForPrincipalCreateAction(
  mountRoot: string,
  clientToken: string,
  principalId: string,
): Promise<AppfsControlEvent> {
  const event = await waitForPrincipalControlAction({
    mountRoot,
    clientToken,
    principalId,
    path: '/_appfs/principals/create_principal.act',
    completedEvent: null,
    timeoutMessage: `Timed out waiting for AppFS to create principal ${principalId}`,
    failureMessage: `AppFS create_principal failed for ${principalId}`,
  });
  const principalEvent = event.content?.principal_event;
  if (principalEvent !== 'principal.created' && principalEvent !== 'principal.exists') {
    throw new PrincipalLifecycleError(
      409,
      `Unexpected principal create event for ${principalId}: ${principalEvent ?? '<missing>'}`,
      'PRINCIPAL_CREATE_UNEXPECTED_EVENT',
    );
  }
  return event;
}

async function waitForPrincipalDetachAction(
  mountRoot: string,
  clientToken: string,
  principalId: string,
  attachId: string,
): Promise<boolean> {
  const event = await waitForPrincipalControlAction({
    mountRoot,
    clientToken,
    principalId,
    path: '/_appfs/principals/detach_principal.act',
    completedEvent: null,
    timeoutMessage: `Timed out waiting for AppFS to detach principal ${principalId}`,
    failureMessage: `AppFS detach_principal failed for ${principalId}`,
  });
  return event.content?.principal_event === 'principal.detached'
    && event.content?.attach_id === attachId;
}

async function waitForPrincipalControlAction({
  mountRoot,
  clientToken,
  principalId,
  path: controlPath,
  completedEvent,
  timeoutMessage,
  failureMessage,
}: {
  mountRoot: string;
  clientToken: string;
  principalId: string;
  path: string;
  completedEvent: string | null;
  timeoutMessage: string;
  failureMessage: string;
}): Promise<AppfsControlEvent> {
  const deadline = Date.now() + APPFS_ACTION_WAIT_TIMEOUT_MS;
  const eventsPath = path.join(mountRoot, '_appfs', '_stream', 'events.evt.jsonl');
  let offset = 0;

  while (Date.now() <= deadline) {
    const result = readMatchingControlEvent(eventsPath, clientToken, principalId, controlPath, offset);
    offset = result.offset;
    if (result.event) {
      const event = result.event;
      if (
        event.type === 'action.completed'
        && event.content?.principal_id === principalId
        && (!completedEvent || event.content.principal_event === completedEvent)
      ) {
        return event;
      }
      if (event.type === 'action.failed') {
        const code = event.error?.code ?? 'APPFS_ACTION_FAILED';
        const message = event.error?.message ?? failureMessage;
        throw new PrincipalLifecycleError(409, `${code}: ${message}`);
      }
    }
    await sleep(APPFS_ACTION_POLL_MS);
  }

  throw new PrincipalLifecycleError(504, timeoutMessage);
}

function readMatchingControlEvent(
  eventsPath: string,
  clientToken: string,
  principalId: string,
  controlPath: string,
  offset: number,
): { event: AppfsControlEvent | null; offset: number } {
  if (!fs.existsSync(eventsPath)) {
    return { event: null, offset };
  }

  const content = fs.readFileSync(eventsPath, 'utf8');
  const nextOffset = content.length;
  const slice = content.slice(Math.min(offset, content.length));
  for (const line of slice.split(/\r?\n/)) {
    if (!line.trim()) {
      continue;
    }
    const event = parseControlEvent(line);
    if (!event) {
      continue;
    }
    if (
      event.client_token === clientToken
      && event.path === controlPath
      && (
        event.content?.principal_id === principalId
        || event.error
        || event.type === 'action.failed'
      )
    ) {
      return { event, offset: nextOffset };
    }
  }
  return { event: null, offset: nextOffset };
}

async function detachPrincipalIfAttached(
  mountRoot: string,
  principalId: string,
): Promise<{ detached: boolean; attachId: string | null; reason: string }> {
  const principal = readPrincipalViews(mountRoot).principals
    .find((item) => item.principal_id === principalId);
  const attach = freshestAttach(principal);
  if (!attach?.attach_id) {
    return { detached: false, attachId: null, reason: 'no-active-attach' };
  }

  const clientToken = appendPrincipalAction(mountRoot, 'detach_principal.act', {
    principal_id: principalId,
    attach_id: attach.attach_id,
    reason: 'dashboard_stop',
  });
  const detached = await waitForPrincipalDetachAction(
    mountRoot,
    clientToken,
    principalId,
    attach.attach_id,
  );
  return {
    detached,
    attachId: attach.attach_id,
    reason: detached ? 'detached' : 'detach-ignored',
  };
}

function parseControlEvent(line: string): AppfsControlEvent | null {
  try {
    const value = JSON.parse(line) as unknown;
    return value && typeof value === 'object' ? value as AppfsControlEvent : null;
  } catch {
    return null;
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function principalAttachState(
  principal: PrincipalRegistryPrincipal,
): { active: boolean; hasStaleAttaches: boolean } {
  const activeAttaches = principal.active_attaches ?? [];
  if (activeAttaches.length > 0) {
    const hasActive = activeAttaches.some((attach) => !isAttachStale(attach.last_seen_at));
    return { active: hasActive, hasStaleAttaches: !hasActive };
  }

  const hasAttachCount = typeof principal.active_attach_count === 'number'
    && principal.active_attach_count > 0;
  if (hasAttachCount) {
    return { active: false, hasStaleAttaches: true };
  }

  return { active: false, hasStaleAttaches: false };
}

function freshestAttach(
  principal: PrincipalRegistryPrincipal | undefined,
): { attach_id?: string; last_seen_at?: string; [key: string]: unknown } | null {
  const activeAttaches = principal?.active_attaches ?? [];
  return activeAttaches
    .filter((attach) => typeof attach.attach_id === 'string' && attach.attach_id.length > 0)
    .sort((left, right) => {
      const leftTime = typeof left.last_seen_at === 'string' ? Date.parse(left.last_seen_at) : 0;
      const rightTime = typeof right.last_seen_at === 'string' ? Date.parse(right.last_seen_at) : 0;
      return (Number.isFinite(rightTime) ? rightTime : 0) - (Number.isFinite(leftTime) ? leftTime : 0);
    })[0] ?? null;
}

function isAttachStale(lastSeenAt: unknown): boolean {
  if (typeof lastSeenAt !== 'string') {
    return true;
  }
  const timestamp = Date.parse(lastSeenAt);
  if (!Number.isFinite(timestamp)) {
    return true;
  }
  return Date.now() - timestamp > ATTACH_STALE_AFTER_MS;
}

function isManagedPrincipalInfo(
  value: ManagedPrincipalInfo | AgentInfo | undefined,
): value is ManagedPrincipalInfo {
  return Boolean(value && 'permissionMode' in value);
}
