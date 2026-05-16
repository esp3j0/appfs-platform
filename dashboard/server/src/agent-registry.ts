import fs from 'node:fs';
import path from 'node:path';
import type { AgentInfo, AgentMeta, MessageRecord, SessionMetaRecord } from './types.js';
import { parseMessages, parseMeta } from './jsonl-parser.js';

export class AgentRegistry {
  private agents = new Map<string, AgentInfo>();
  private messages = new Map<string, MessageRecord[]>();
  private dumpDir: string;

  constructor(dumpDir: string) {
    this.dumpDir = dumpDir;
  }

  /** Scan for agents. Discovery order:
   *  1. agent-meta-*.json files in dump dir (Phase 3 debug-dump)
   *  2. .claw/sessions/<fingerprint>/*.jsonl (real claw session layout)
   *  3. *.jsonl directly in dump dir (flat fixture mode) */
  discover(): void {
    if (!fs.existsSync(this.dumpDir)) return;

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
      return;
    }

    // Phase 1b: .claw/sessions/<fingerprint>/*.jsonl
    const clawSessionsDir = path.join(this.dumpDir, '.claw', 'sessions');
    if (fs.existsSync(clawSessionsDir)) {
      this.discoverClawSessions(clawSessionsDir);
      if (this.agents.size > 0) return;
    }

    // Phase 1a fallback: flat *.jsonl files in dump dir
    const jsonlFiles = fs.readdirSync(this.dumpDir).filter(f => f.endsWith('.jsonl'));
    for (const file of jsonlFiles) {
      const fullPath = path.join(this.dumpDir, file);
      this.registerFromSessionFile(fullPath);
    }
  }

  /** Recursively scan .claw/sessions/ for session-*.jsonl files.
   *  Layout: .claw/sessions/<fingerprint>/<session-id>.jsonl
   *  Both agents (default, code-implementer) share the same fingerprint dir
   *  because they share the same workspace path. */
  private discoverClawSessions(sessionsDir: string): void {
    // Read fingerprint directories
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
        sessionFiles = fs.readdirSync(fpDir).filter(f => f.endsWith('.jsonl'));
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
    const name = meta.agent_name;
    let sessionContent = '';
    if (meta.session_jsonl_path && fs.existsSync(meta.session_jsonl_path)) {
      sessionContent = fs.readFileSync(meta.session_jsonl_path, 'utf-8');
    }
    const msgs = parseMessages(sessionContent);

    this.agents.set(name, {
      name,
      principalId: meta.principal_id,
      sessionId: meta.session_id,
      model: meta.model,
      pid: meta.pid,
      startedAt: meta.started_at_ms,
      sessionJsonlPath: meta.session_jsonl_path,
      status: 'online',
      messageCount: msgs.length,
      ...this.sumUsage(msgs),
    });
    this.messages.set(name, msgs);
  }

  private registerFromSessionFile(fullPath: string): void {
    const content = fs.readFileSync(fullPath, 'utf-8');
    const sess = parseMeta(content);
    const msgs = parseMessages(content);
    // Use principal_id as the display name when available, fallback to session_id
    const principalId = sess?.appfs_principal_id;
    const name = principalId ?? sess?.session_id ?? path.basename(fullPath, '.jsonl');

    this.agents.set(name, {
      name,
      principalId: principalId ?? name,
      sessionId: sess?.session_id ?? name,
      model: sess?.model ?? 'unknown',
      pid: 0,
      startedAt: sess?.created_at_ms ?? Date.now(),
      sessionJsonlPath: fullPath,
      status: 'online',
      messageCount: msgs.length,
      ...this.sumUsage(msgs),
    });
    this.messages.set(name, msgs);
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
    return Array.from(this.agents.values());
  }

  getAgent(name: string): AgentInfo | undefined {
    return this.agents.get(name);
  }

  getMessages(name: string): MessageRecord[] {
    return this.messages.get(name) ?? [];
  }

  /** Reload a single agent's messages from its session file. */
  reloadAgent(name: string): MessageRecord[] {
    const info = this.agents.get(name);
    if (!info) return [];
    const content = fs.readFileSync(info.sessionJsonlPath, 'utf-8');
    const msgs = parseMessages(content);
    this.messages.set(name, msgs);
    this.agents.set(name, {
      ...info,
      messageCount: msgs.length,
      ...this.sumUsage(msgs),
    });
    return msgs;
  }

  get dumpDirectory(): string {
    return this.dumpDir;
  }

  getSessionPaths(): string[] {
    return Array.from(this.agents.values()).map(a => a.sessionJsonlPath).filter(Boolean);
  }
}
