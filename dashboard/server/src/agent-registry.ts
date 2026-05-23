import fs from 'node:fs';
import path from 'node:path';
import type { AgentInfo, AgentMeta, CompactionArchiveRecord, CompactionBoundaryRecord, DebugDumpRecord, MessageRecord, SessionMetaRecord } from './types.js';
import { parseCompactionArchives, parseCompactionBoundaries, parseDebugDumps, parseMessages, parseMeta } from './jsonl-parser.js';
import type { FileWatcher } from './file-watcher.js';

export class AgentRegistry {
  private agents = new Map<string, AgentInfo>();
  private messages = new Map<string, MessageRecord[]>();
  private debugDumps = new Map<string, DebugDumpRecord[]>();
  private compactionArchives = new Map<string, CompactionArchiveRecord[]>();
  private compactionBoundaries = new Map<string, CompactionBoundaryRecord[]>();
  private dumpDir: string;
  private fileWatcher: FileWatcher | null = null;

  constructor(dumpDir: string) {
    this.dumpDir = dumpDir;
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
        const jsonlFiles = fs.readdirSync(this.dumpDir).filter(f => f.endsWith('.jsonl') && !f.endsWith('.debug.jsonl'));
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
        sessionFiles = fs.readdirSync(fpDir).filter(f => f.endsWith('.jsonl') && !f.endsWith('.debug.jsonl'));
      } catch {
        continue;
      }

      for (const file of sessionFiles) {
        const fullPath = path.join(fpDir, file);
        this.registerFromSessionFile(fullPath);
      }
    }
  }

  private registerFromMeta(meta: AgentMeta): void {
    const sessionId = meta.session_id;
    let sessionContent = '';
    if (meta.session_jsonl_path && fs.existsSync(meta.session_jsonl_path)) {
      sessionContent = fs.readFileSync(meta.session_jsonl_path, 'utf-8');
    }
    const msgs = parseMessages(sessionContent);
    // Look for companion .debug.jsonl
    const debugPath = (meta.session_jsonl_path ?? '').replace(/\.jsonl$/, '.debug.jsonl');
    let dumps: DebugDumpRecord[] = [];
    let archives: CompactionArchiveRecord[] = [];
    let boundaries: CompactionBoundaryRecord[] = [];
    if (debugPath && fs.existsSync(debugPath)) {
      const debugContent = fs.readFileSync(debugPath, 'utf-8');
      dumps = parseDebugDumps(debugContent);
      archives = parseCompactionArchives(debugContent);
      boundaries = parseCompactionBoundaries(debugContent);
    }

    const name = meta.agent_name;
    this.agents.set(sessionId, {
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
      ...this.sumUsage(msgs),
    });
    this.messages.set(sessionId, msgs);
    this.debugDumps.set(sessionId, dumps);
    this.compactionArchives.set(sessionId, archives);
    this.compactionBoundaries.set(sessionId, boundaries);
  }

  private registerFromSessionFile(fullPath: string): void {
    const content = fs.readFileSync(fullPath, 'utf-8');
    const sess = parseMeta(content);
    const msgs = parseMessages(content);
    // Look for companion .debug.jsonl file
    const debugPath = fullPath.replace(/\.jsonl$/, '.debug.jsonl');
    let dumps: DebugDumpRecord[] = [];
    let archives: CompactionArchiveRecord[] = [];
    let boundaries: CompactionBoundaryRecord[] = [];
    if (fs.existsSync(debugPath)) {
      const debugContent = fs.readFileSync(debugPath, 'utf-8');
      dumps = parseDebugDumps(debugContent);
      archives = parseCompactionArchives(debugContent);
      boundaries = parseCompactionBoundaries(debugContent);
    }

    const principalId = sess?.appfs_principal_id;
    const name = principalId ?? sess?.session_id ?? path.basename(fullPath, '.jsonl');
    const sessionId = sess?.session_id ?? name;

    this.agents.set(sessionId, {
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
      ...this.sumUsage(msgs),
    });
    this.messages.set(sessionId, msgs);
    this.debugDumps.set(sessionId, dumps);
    this.compactionArchives.set(sessionId, archives);
    this.compactionBoundaries.set(sessionId, boundaries);
  }

  /**
   * Directly register or update an agent in the registry.
   * Useful when spawning a managed agent process.
   */
  registerAgent(agentInfo: AgentInfo, msgs?: MessageRecord[]): void {
    const sessionId = agentInfo.sessionId;
    this.agents.set(sessionId, agentInfo);
    if (msgs) {
      this.messages.set(sessionId, msgs);
    } else if (!this.messages.has(sessionId) && agentInfo.sessionJsonlPath && fs.existsSync(agentInfo.sessionJsonlPath)) {
      this.reloadAgent(sessionId);
    } else if (!this.messages.has(sessionId)) {
      this.messages.set(sessionId, []);
    }
    if (!this.debugDumps.has(sessionId)) this.debugDumps.set(sessionId, []);
    if (!this.compactionArchives.has(sessionId)) this.compactionArchives.set(sessionId, []);
    if (!this.compactionBoundaries.has(sessionId)) this.compactionBoundaries.set(sessionId, []);

    // Dynamically watch the file if a watcher exists
    if (this.fileWatcher && agentInfo.sessionJsonlPath) {
      this.fileWatcher.addPath(agentInfo.sessionJsonlPath);
    }
  }

  private sumUsage(msgs: MessageRecord[]): { totalInputTokens: number; totalOutputTokens: number } {
    let totalInputTokens = 0;
    let totalOutputTokens = 0;
    for (const msg of msgs) {
      if (msg.message.usage) {
        totalInputTokens += msg.message.usage.input_tokens;
        totalOutputTokens += msg.message.usage.output_tokens;
      }
    }
    return { totalInputTokens, totalOutputTokens };
  }

  getAgents(): AgentInfo[] {
    for (const sessionId of Array.from(this.agents.keys())) {
      this.reloadAgent(sessionId);
    }
    return Array.from(this.agents.values());
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

  /** Reload a single agent's messages and debug dumps from its session file. */
  reloadAgent(key: string): MessageRecord[] {
    const sessionId = this.resolveSessionId(key);
    if (!sessionId) return [];
    const info = this.agents.get(sessionId);
    if (!info || !info.sessionJsonlPath || !fs.existsSync(info.sessionJsonlPath)) return [];

    try {
      const content = fs.readFileSync(info.sessionJsonlPath, 'utf-8');
      const msgs = parseMessages(content);
      const debugPath = info.sessionJsonlPath.replace(/\.jsonl$/, '.debug.jsonl');
      let dumps: DebugDumpRecord[] = [];
      let archives: CompactionArchiveRecord[] = [];
      let boundaries: CompactionBoundaryRecord[] = [];
      if (fs.existsSync(debugPath)) {
        const debugContent = fs.readFileSync(debugPath, 'utf-8');
        dumps = parseDebugDumps(debugContent);
        archives = parseCompactionArchives(debugContent);
        boundaries = parseCompactionBoundaries(debugContent);
      }
      this.messages.set(sessionId, msgs);
      this.debugDumps.set(sessionId, dumps);
      this.compactionArchives.set(sessionId, archives);
      this.compactionBoundaries.set(sessionId, boundaries);
      this.agents.set(sessionId, {
        ...info,
        messageCount: msgs.length,
        ...this.sumUsage(msgs),
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
