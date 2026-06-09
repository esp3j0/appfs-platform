import fs from 'node:fs';
import path from 'node:path';
import type { AgentInfo, AgentMeta, CompactionArchiveRecord, CompactionBoundaryRecord, ConversationMessage, DebugDumpRecord, MessageRecord, SessionMetaRecord, TurnErrorRecord } from './types.js';
import { parseCompactionArchives, parseCompactionBoundaries, parseDebugDumps, parseMessages, parseMeta, parseTurnErrors } from './jsonl-parser.js';
import type { FileWatcher } from './file-watcher.js';
import type { ProjectRegistry, ProjectRecord } from './project-registry.js';

export class AgentRegistry {
  private agents = new Map<string, AgentInfo>();
  private messages = new Map<string, MessageRecord[]>();
  private debugDumps = new Map<string, DebugDumpRecord[]>();
  private compactionArchives = new Map<string, CompactionArchiveRecord[]>();
  private compactionBoundaries = new Map<string, CompactionBoundaryRecord[]>();
  private turnErrors = new Map<string, TurnErrorRecord[]>();
  private dumpDir: string;
  private fileWatcher: FileWatcher | null = null;
  public projectRegistry: ProjectRegistry;

  constructor(dumpDir: string, projectRegistry: ProjectRegistry) {
    this.dumpDir = dumpDir;
    this.projectRegistry = projectRegistry;
  }

  setFileWatcher(watcher: FileWatcher): void {
    this.fileWatcher = watcher;
  }

  getFileWatcher(): FileWatcher | null {
    return this.fileWatcher;
  }

  /**
   * Helper to resolve the correct sessionId key from a lookup key
   * which could be a sessionId or legacy agent name.
   */
  private resolveSessionId(key: string): string | null {
    if (this.agents.has(key)) return key;
    // Fallback: search by name
    const agent = Array.from(this.agents.values()).find(a => a.name === key);
    if (agent) return agent.sessionId;
    return null;
  }

  /**
   * External wrapper to perform on-demand directory scan and automatically
   * add any newly found session paths to the active FileWatcher.
   */
  rediscover(): void {
    this.discover();
  }

  /** Scan a project root for persisted claw sessions. Used by desktop mode,
   * where the dashboard starts with an empty registry and projects are opened
   * after the backend is already running. */
  discoverProject(projectRoot: string): void {
    this.discoverSessionRoot(projectRoot, { flatFallback: false });
  }

  /** Scan for agents. Discovery order:
   *  1. agent-meta-*.json files in dump dir (Phase 3 debug-dump)
   *  2. .claw/sessions/<fingerprint>/*.jsonl (real claw session layout)
   *  3. *.jsonl directly in dump dir (flat fixture mode) */
  discover(): void {
    if (!fs.existsSync(this.dumpDir)) return;

    const oldPaths = new Set(this.getSessionPaths());

    // Phase 3 path: agent-meta.json files exist in dump dir
    const metaFiles = fs.readdirSync(this.dumpDir).filter(f => f.startsWith('agent-meta'));

    if (metaFiles.length > 0) {
      for (const file of metaFiles) {
        try {
          const meta: AgentMeta = JSON.parse(
            fs.readFileSync(path.join(this.dumpDir, file), 'utf-8'),
          );
          this.registerFromMeta(meta);
        } catch {
          // Skip invalid meta files
        }
      }
    } else {
      // Phase 1b: .claw/sessions/<fingerprint>/*.jsonl
      const clawSessionsDir = path.join(this.dumpDir, '.claw', 'sessions');
      if (fs.existsSync(clawSessionsDir)) {
        this.discoverClawSessions(clawSessionsDir);
      }

      // Phase 1a fallback: flat *.jsonl files in dump dir if no sessions were discovered in .claw
      if (this.agents.size === 0) {
        const jsonlFiles = fs.readdirSync(this.dumpDir).filter(isPrimarySessionJsonlFile);
        for (const file of jsonlFiles) {
          const fullPath = path.join(this.dumpDir, file);
          this.registerFromSessionFile(fullPath);
        }
      }
    }

    // Proactively register new paths in the file watcher if it exists
    if (this.fileWatcher) {
      const newPaths = this.getSessionPaths();
      for (const p of newPaths) {
        if (!oldPaths.has(p)) {
          this.fileWatcher.addPath(p);
        }
      }
    }
  }

  private discoverSessionRoot(root: string, options: { flatFallback: boolean }): void {
    if (!root || !fs.existsSync(root)) return;

    const oldPaths = new Set(this.getSessionPaths());
    const oldAgentCount = this.agents.size;

    const clawSessionsDir = path.join(root, '.claw', 'sessions');
    if (fs.existsSync(clawSessionsDir)) {
      this.discoverClawSessions(clawSessionsDir);
    }

    if (options.flatFallback && this.agents.size === oldAgentCount) {
      let jsonlFiles: string[];
      try {
        jsonlFiles = fs.readdirSync(root).filter(isPrimarySessionJsonlFile);
      } catch {
        jsonlFiles = [];
      }
      for (const file of jsonlFiles) {
        this.registerFromSessionFile(path.join(root, file));
      }
    }

    if (this.fileWatcher) {
      for (const p of this.getSessionPaths()) {
        if (!oldPaths.has(p)) {
          this.fileWatcher.addPath(p);
        }
      }
    }
  }

  /** Recursively scan .claw/sessions/ for session-*.jsonl files.
   *  Layout: .claw/sessions/<fingerprint>/<session-id>.jsonl */
  private discoverClawSessions(sessionsDir: string): void {
    let entries: string[];
    try {
      entries = fs.readdirSync(sessionsDir);
    } catch {
      return;
    }

    for (const fingerprint of entries) {
      const fpDir = path.join(sessionsDir, fingerprint);
      if (!fs.statSync(fpDir).isDirectory()) continue;

      let sessionFiles: string[];
      try {
        sessionFiles = fs.readdirSync(fpDir).filter(isPrimarySessionJsonlFile);
      } catch {
        continue;
      }

      for (const file of sessionFiles) {
        const fullPath = path.join(fpDir, file);
        this.registerFromSessionFile(fullPath);
      }
    }
  }

  private readSessionMessages(sessionPath: string): MessageRecord[] {
    const order: string[] = [];
    const recordsByKey = new Map<string, MessageRecord>();
    const paths = [...findRotatedSessionPaths(sessionPath), sessionPath];

    for (const filePath of paths) {
      if (!fs.existsSync(filePath)) continue;
      const content = fs.readFileSync(filePath, 'utf-8');
      for (const record of parseMessages(content)) {
        const key = record.message.uuid || `${filePath}:${order.length}`;
        if (!recordsByKey.has(key)) {
          order.push(key);
        }
        recordsByKey.set(key, record);
      }
    }

    return order.flatMap(key => {
      const record = recordsByKey.get(key);
      return record ? [record] : [];
    });
  }

  private readDebugSidecar(sessionPath: string): {
    dumps: DebugDumpRecord[];
    archives: CompactionArchiveRecord[];
    boundaries: CompactionBoundaryRecord[];
    errors: TurnErrorRecord[];
  } {
    const debugPath = sessionPath.replace(/\.jsonl$/, '.debug.jsonl');
    if (!debugPath || !fs.existsSync(debugPath)) {
      return { dumps: [], archives: [], boundaries: [], errors: [] };
    }

    const debugContent = fs.readFileSync(debugPath, 'utf-8');
    return {
      dumps: parseDebugDumps(debugContent),
      archives: parseCompactionArchives(debugContent),
      boundaries: parseCompactionBoundaries(debugContent),
      errors: parseTurnErrors(debugContent),
    };
  }

  private registerFromMeta(meta: AgentMeta): void {
    const sessionId = meta.session_id;
    const msgs = meta.session_jsonl_path
      ? this.readSessionMessages(meta.session_jsonl_path)
      : [];
    const debug = meta.session_jsonl_path
      ? this.readDebugSidecar(meta.session_jsonl_path)
      : { dumps: [], archives: [], boundaries: [], errors: [] };

    const name = meta.agent_name;
    const sidecarMeta = meta.session_jsonl_path
      ? this.readSessionSidecarMeta(meta.session_jsonl_path)
      : {};
    const agentInfo: AgentInfo = {
      name,
      principalId: meta.principal_id,
      sessionId,
      workspaceFingerprint: workspaceFingerprintFromSessionPath(meta.session_jsonl_path),
      model: meta.model,
      pid: meta.pid,
      startedAt: meta.started_at_ms,
      sessionJsonlPath: meta.session_jsonl_path,
      status: 'online',
      controlMode: 'external', // Managed agents will override this explicitly
      messageCount: msgs.length,
      ...calculateSessionUsage(msgs, debug.archives),
      modelProviderId: stringValue(sidecarMeta.modelProviderId),
      modelId: stringValue(sidecarMeta.modelId),
      contextWindowTokens: numberValue(sidecarMeta.contextWindowTokens),
      maxOutputTokens: numberValue(sidecarMeta.maxOutputTokens),
      runtimeModelConfigPath: stringValue(sidecarMeta.runtimeModelConfigPath),
      archived: sidecarMeta.archived === true,
      archivedAt: numberValue(sidecarMeta.archivedAt),
      archivedReason: stringValue(sidecarMeta.archivedReason),
    };
    const normalizedAgentInfo = this.preserveManagedRuntimeState(agentInfo);
    this.fillProjectInfo(normalizedAgentInfo);
    this.agents.set(sessionId, normalizedAgentInfo);
    this.messages.set(sessionId, msgs);
    this.debugDumps.set(sessionId, debug.dumps);
    this.compactionArchives.set(sessionId, debug.archives);
    this.compactionBoundaries.set(sessionId, debug.boundaries);
    this.turnErrors.set(sessionId, debug.errors);
  }

  private registerFromSessionFile(fullPath: string): void {
    const content = fs.readFileSync(fullPath, 'utf-8');
    const sess = parseMeta(content);
    const msgs = this.readSessionMessages(fullPath);
    const debug = this.readDebugSidecar(fullPath);

    const principalId = sess?.appfs_principal_id;
    const name = principalId ?? sess?.session_id ?? path.basename(fullPath, '.jsonl');
    const sessionId = sess?.session_id ?? name;

    const sidecarMeta = this.readSessionSidecarMeta(fullPath);

    const agentInfo: AgentInfo = {
      name,
      principalId: principalId ?? name,
      sessionId,
      workspaceFingerprint: workspaceFingerprintFromSessionPath(fullPath),
      model: sess?.model ?? 'unknown',
      pid: 0,
      startedAt: sess?.created_at_ms ?? Date.now(),
      sessionJsonlPath: fullPath,
      status: 'offline',
      controlMode: 'external',
      messageCount: msgs.length,
      ...calculateSessionUsage(msgs, debug.archives),
      modelProviderId: stringValue(sidecarMeta.modelProviderId),
      modelId: stringValue(sidecarMeta.modelId),
      contextWindowTokens: numberValue(sidecarMeta.contextWindowTokens),
      maxOutputTokens: numberValue(sidecarMeta.maxOutputTokens),
      runtimeModelConfigPath: stringValue(sidecarMeta.runtimeModelConfigPath),
      archived: sidecarMeta.archived === true,
      archivedAt: numberValue(sidecarMeta.archivedAt),
      archivedReason: stringValue(sidecarMeta.archivedReason),
    };
    const normalizedAgentInfo = this.preserveManagedRuntimeState(agentInfo);
    this.fillProjectInfo(normalizedAgentInfo);
    this.agents.set(sessionId, normalizedAgentInfo);
    this.messages.set(sessionId, msgs);
    this.debugDumps.set(sessionId, debug.dumps);
    this.compactionArchives.set(sessionId, debug.archives);
    this.compactionBoundaries.set(sessionId, debug.boundaries);
    this.turnErrors.set(sessionId, debug.errors);
  }

  /**
   * Directly register or update an agent in the registry.
   * Useful when spawning a managed agent process.
   */
  registerAgent(agentInfo: AgentInfo, msgs?: MessageRecord[]): void {
    const sessionId = agentInfo.sessionId;
    const normalizedAgentInfo: AgentInfo = { ...agentInfo };
    this.fillProjectInfo(normalizedAgentInfo);
    this.agents.set(sessionId, normalizedAgentInfo);

    // Save sidecar meta if managed and has a sessionJsonlPath
    if (normalizedAgentInfo.sessionJsonlPath && normalizedAgentInfo.controlMode === 'managed') {
      this.writeSessionSidecarMeta(normalizedAgentInfo);
    }

    if (msgs) {
      this.messages.set(sessionId, msgs);
    } else if (!this.messages.has(sessionId) && normalizedAgentInfo.sessionJsonlPath && fs.existsSync(normalizedAgentInfo.sessionJsonlPath)) {
      this.reloadAgent(sessionId);
    } else if (!this.messages.has(sessionId)) {
      this.messages.set(sessionId, []);
    }
    if (!this.debugDumps.has(sessionId)) this.debugDumps.set(sessionId, []);
    if (!this.compactionArchives.has(sessionId)) this.compactionArchives.set(sessionId, []);
    if (!this.compactionBoundaries.has(sessionId)) this.compactionBoundaries.set(sessionId, []);
    if (!this.turnErrors.has(sessionId)) this.turnErrors.set(sessionId, []);

    // Dynamically watch the file if a watcher exists
    if (this.fileWatcher && normalizedAgentInfo.sessionJsonlPath) {
      this.fileWatcher.addPath(normalizedAgentInfo.sessionJsonlPath);
    }
  }

  private preserveManagedRuntimeState(discovered: AgentInfo): AgentInfo {
    const existing = this.agents.get(discovered.sessionId);
    if (existing?.controlMode !== 'managed') {
      return discovered;
    }

    return {
      ...discovered,
      name: existing.name || discovered.name,
      principalId: existing.principalId || discovered.principalId,
      pid: existing.pid,
      startedAt: existing.startedAt || discovered.startedAt,
      status: existing.status,
      controlMode: existing.controlMode,
      projectId: existing.projectId ?? discovered.projectId,
      projectRoot: existing.projectRoot ?? discovered.projectRoot,
      modelProviderId: discovered.modelProviderId ?? existing.modelProviderId,
      modelId: discovered.modelId ?? existing.modelId,
      contextWindowTokens: discovered.contextWindowTokens ?? existing.contextWindowTokens,
      maxOutputTokens: discovered.maxOutputTokens ?? existing.maxOutputTokens,
      runtimeModelConfigPath: discovered.runtimeModelConfigPath ?? existing.runtimeModelConfigPath,
      archived: discovered.archived ?? existing.archived,
      archivedAt: discovered.archivedAt ?? existing.archivedAt,
      archivedReason: discovered.archivedReason ?? existing.archivedReason,
    };
  }

  getAgents(): AgentInfo[] {
    for (const sessionId of Array.from(this.agents.keys())) {
      this.reloadAgent(sessionId);
    }
    return Array.from(this.agents.values());
  }

  getActiveAgents(): AgentInfo[] {
    return this.getAgents().filter(agent => !agent.archived);
  }

  getArchivedAgents(): AgentInfo[] {
    return this.getAgents().filter(agent => agent.archived);
  }

  archiveSessionsForPrincipal(
    principalId: string,
    projectId?: string,
    reason = 'principal_deleted',
  ): AgentInfo[] {
    const archived: AgentInfo[] = [];
    const archivedAt = Date.now();

    for (const agent of this.getAgents()) {
      if (!samePrincipal(agent, principalId, projectId) || agent.archived) {
        continue;
      }

      const updated: AgentInfo = {
        ...agent,
        archived: true,
        archivedAt,
        archivedReason: reason,
      };
      this.agents.set(agent.sessionId, updated);
      this.writeSessionSidecarMeta(updated);
      archived.push(updated);
    }

    return archived;
  }

  getAgent(key: string): AgentInfo | undefined {
    const sessionId = this.resolveSessionId(key);
    return sessionId ? this.agents.get(sessionId) : undefined;
  }

  getMessages(key: string): MessageRecord[] {
    const sessionId = this.resolveSessionId(key);
    if (sessionId) {
      this.reloadAgent(sessionId);
    }
    return sessionId ? (this.messages.get(sessionId) ?? []) : [];
  }

  getDebugDumps(key: string): DebugDumpRecord[] {
    const sessionId = this.resolveSessionId(key);
    if (sessionId) {
      this.reloadAgent(sessionId);
    }
    return sessionId ? (this.debugDumps.get(sessionId) ?? []) : [];
  }

  getCompactionArchives(key: string): CompactionArchiveRecord[] {
    const sessionId = this.resolveSessionId(key);
    if (sessionId) {
      this.reloadAgent(sessionId);
    }
    return sessionId ? (this.compactionArchives.get(sessionId) ?? []) : [];
  }

  getCompactionBoundaries(key: string): CompactionBoundaryRecord[] {
    const sessionId = this.resolveSessionId(key);
    if (sessionId) {
      this.reloadAgent(sessionId);
    }
    return sessionId ? (this.compactionBoundaries.get(sessionId) ?? []) : [];
  }

  getTurnErrors(key: string): TurnErrorRecord[] {
    const sessionId = this.resolveSessionId(key);
    if (sessionId) {
      this.reloadAgent(sessionId);
    }
    return sessionId ? (this.turnErrors.get(sessionId) ?? []) : [];
  }

  /** Reload a single agent's messages and debug dumps from its session file. */
  reloadAgent(key: string): MessageRecord[] {
    const sessionId = this.resolveSessionId(key);
    if (!sessionId) return [];
    const info = this.agents.get(sessionId);
    if (!info || !info.sessionJsonlPath || !fs.existsSync(info.sessionJsonlPath)) return [];

    try {
      const msgs = this.readSessionMessages(info.sessionJsonlPath);
      const debug = this.readDebugSidecar(info.sessionJsonlPath);
      this.messages.set(sessionId, msgs);
      this.debugDumps.set(sessionId, debug.dumps);
      this.compactionArchives.set(sessionId, debug.archives);
      this.compactionBoundaries.set(sessionId, debug.boundaries);
      this.turnErrors.set(sessionId, debug.errors);
      this.agents.set(sessionId, {
        ...info,
        messageCount: msgs.length,
        ...calculateSessionUsage(msgs, debug.archives),
      });
      return msgs;
    } catch (err) {
      console.error(`Error reloading agent ${sessionId}:`, err);
      return [];
    }
  }

  get dumpDirectory(): string {
    return this.dumpDir;
  }

  getSessionPaths(): string[] {
    return Array.from(this.agents.values()).map(a => a.sessionJsonlPath).filter(Boolean);
  }

  private fillProjectInfo(agentInfo: AgentInfo): void {
    const sessionId = agentInfo.sessionId;
    const oldAgent = this.agents.get(sessionId);
    const oldProjectId = oldAgent?.projectId;

    let targetProject: ProjectRecord | undefined = undefined;

    // 1. If projectId exists, check if it exists in projectRegistry
    if (agentInfo.projectId) {
      const proj = this.projectRegistry.getProject(agentInfo.projectId);
      if (proj) {
        targetProject = proj;
      }
    }

    // 2. If targetProject not found, try to infer from projectRoot
    if (!targetProject && agentInfo.projectRoot) {
      try {
        targetProject = this.projectRegistry.registerProject(agentInfo.projectRoot);
      } catch {
        targetProject = this.projectRegistry.getProjectByRoot(agentInfo.projectRoot);
      }
    }

    // 3. If targetProject still not found, try to infer from sessionJsonlPath
    if (!targetProject && agentInfo.sessionJsonlPath) {
      const normalizedPath = path.resolve(agentInfo.sessionJsonlPath).replace(/\\/g, '/');
      const clawIndex = normalizedPath.lastIndexOf('/.claw/sessions/');
      if (clawIndex !== -1) {
        const inferredProjectRoot = path.resolve(normalizedPath.substring(0, clawIndex));
        try {
          targetProject = this.projectRegistry.registerProject(inferredProjectRoot);
        } catch {
          targetProject = this.projectRegistry.getProjectByRoot(inferredProjectRoot);
        }
      }
    }

    // 4. Update the relationship
    if (targetProject) {
      const newProjectId = targetProject.projectId;

      // If project changed, detach from old project
      if (oldProjectId && oldProjectId !== newProjectId) {
        this.projectRegistry.detachAgent(oldProjectId, sessionId);
      }

      agentInfo.projectId = newProjectId;
      agentInfo.projectRoot = targetProject.projectRoot;
      this.projectRegistry.attachAgent(newProjectId, sessionId, agentInfo.controlMode);
    } else {
      // If we failed to map, and there was an old project, detach it
      if (oldProjectId) {
        this.projectRegistry.detachAgent(oldProjectId, sessionId);
      }
      // Ensure we don't carry any stale projectId or projectRoot
      delete agentInfo.projectId;
      delete agentInfo.projectRoot;
    }
  }

  private readSessionSidecarMeta(sessionPath: string): Record<string, unknown> {
    const metaPath = sessionPath + '.meta.json';
    if (!sessionPath || !fs.existsSync(metaPath)) {
      return {};
    }
    try {
      const parsed = JSON.parse(fs.readFileSync(metaPath, 'utf-8'));
      return parsed && typeof parsed === 'object' ? parsed : {};
    } catch {
      return {};
    }
  }

  private writeSessionSidecarMeta(agentInfo: AgentInfo): void {
    if (!agentInfo.sessionJsonlPath) {
      return;
    }

    const metaPath = agentInfo.sessionJsonlPath + '.meta.json';
    const existing = this.readSessionSidecarMeta(agentInfo.sessionJsonlPath);
    const sidecarMeta = {
      ...existing,
      modelProviderId: agentInfo.modelProviderId,
      modelId: agentInfo.modelId,
      contextWindowTokens: agentInfo.contextWindowTokens,
      maxOutputTokens: agentInfo.maxOutputTokens,
      runtimeModelConfigPath: agentInfo.runtimeModelConfigPath,
      archived: agentInfo.archived ?? existing.archived,
      archivedAt: agentInfo.archivedAt ?? existing.archivedAt,
      archivedReason: agentInfo.archivedReason ?? existing.archivedReason,
    };

    try {
      fs.mkdirSync(path.dirname(metaPath), { recursive: true });
      fs.writeFileSync(metaPath, JSON.stringify(sidecarMeta, null, 2), 'utf-8');
    } catch (err) {
      console.error(`Failed to write sidecar meta file ${metaPath}:`, err);
    }
  }
}

function samePrincipal(agent: AgentInfo, principalId: string, projectId?: string): boolean {
  return (agent.principalId || agent.name) === principalId
    && (!projectId || agent.projectId === projectId);
}

function stringValue(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

function numberValue(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
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

function isPrimarySessionJsonlFile(file: string): boolean {
  return file.endsWith('.jsonl')
    && !file.endsWith('.debug.jsonl')
    && !/\.rot-\d+\.jsonl$/.test(file);
}

function findRotatedSessionPaths(sessionPath: string): string[] {
  const dir = path.dirname(sessionPath);
  const base = path.basename(sessionPath, '.jsonl');
  if (!fs.existsSync(dir)) return [];

  let files: string[];
  try {
    files = fs.readdirSync(dir);
  } catch {
    return [];
  }

  return files
    .filter(file => file.startsWith(`${base}.rot-`) && file.endsWith('.jsonl'))
    .sort((a, b) => rotationTimestamp(a) - rotationTimestamp(b))
    .map(file => path.join(dir, file));
}

function rotationTimestamp(fileName: string): number {
  const match = fileName.match(/\.rot-(\d+)\.jsonl$/);
  return match ? Number(match[1]) : 0;
}

export function calculateSessionUsage(
  msgs: MessageRecord[],
  archives: CompactionArchiveRecord[] = [],
): { totalInputTokens: number; totalOutputTokens: number; currentContextTokens: number } {
  const liveMessages = msgs.map(record => record.message);
  const archivedMessages = archives.map(record => record.message);
  const usageMessages = uniqueMessages([...archivedMessages, ...liveMessages]);

  return {
    totalInputTokens: sumInputTokens(usageMessages),
    totalOutputTokens: sumOutputTokens(usageMessages),
    currentContextTokens: latestInputTokens(liveMessages),
  };
}

function sumInputTokens(messages: ConversationMessage[]): number {
  let total = 0;
  for (const message of messages) {
    if (message.usage) {
      total += effectiveInputTokens(message.usage);
    }
  }
  return total;
}

function sumOutputTokens(messages: ConversationMessage[]): number {
  let total = 0;
  for (const message of messages) {
    if (message.usage) {
      total += message.usage.output_tokens;
    }
  }
  return total;
}

function latestInputTokens(messages: ConversationMessage[]): number {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const usage = messages[i].usage;
    if (usage) {
      return effectiveInputTokens(usage);
    }
  }
  return 0;
}

function effectiveInputTokens(usage: ConversationMessage['usage']): number {
  if (!usage) return 0;
  return positiveTokenCount(usage.input_tokens)
    + positiveTokenCount(usage.cache_creation_input_tokens)
    + positiveTokenCount(usage.cache_read_input_tokens);
}

function positiveTokenCount(value: number | undefined): number {
  return typeof value === 'number' && value > 0 ? value : 0;
}

function uniqueMessages(messages: ConversationMessage[]): ConversationMessage[] {
  const seen = new Set<string>();
  const unique: ConversationMessage[] = [];
  for (const message of messages) {
    if (!message.uuid) {
      unique.push(message);
      continue;
    }
    if (seen.has(message.uuid)) {
      continue;
    }
    seen.add(message.uuid);
    unique.push(message);
  }
  return unique;
}
