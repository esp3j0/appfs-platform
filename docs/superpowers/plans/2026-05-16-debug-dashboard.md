# AppFS Debug Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone React + Fastify dashboard that reads agent session JSONL files (and optional debug-dump files) to visualise multi-agent LLM I/O in a merged timeline.

**Architecture:** The dashboard lives in `dashboard/` as an independent project. A Node.js server watches JSONL files written by agent processes and exposes REST + SSE endpoints. A React frontend (Vite) renders a three-panel layout: agent sidebar, merged message timeline, and info panel. Agent code gets a small `debug-dump` feature flag (~30 lines) that writes raw MessageRequest data to a JSONL file the server also reads.

**Tech Stack:** Node.js 20+, TypeScript, Fastify, chokidar, React 19, Vite, CSS modules

**Design Spec:** `docs/superpowers/specs/2026-05-16-debug-dashboard-design.md`

---

## File Map

### New files created in `dashboard/`

| File | Responsibility |
|------|---------------|
| `package.json` | Root monorepo scripts (`dev`, `build`, `start`) |
| `index.html` | Vite HTML entry point |
| `vite.config.ts` | Vite config with React plugin + dev proxy to server |
| `tsconfig.json` | Frontend TS config |
| `server/package.json` | Server dependencies |
| `server/tsconfig.json` | Server TS config |
| `server/src/types.ts` | Shared TypeScript interfaces matching session JSONL format |
| `server/src/jsonl-parser.ts` | Streaming JSONL line parser |
| `server/src/agent-registry.ts` | Discover agents from dump-dir meta files or session dirs |
| `server/src/file-watcher.ts` | chokidar wrapper, emit parsed records on change |
| `server/src/routes/agents.ts` | GET /api/agents |
| `server/src/routes/messages.ts` | GET /api/agents/:name/messages |
| `server/src/routes/timeline.ts` | GET /api/timeline?agents=a,b |
| `server/src/routes/events.ts` | GET /api/events (SSE) |
| `server/src/index.ts` | Fastify entry, CLI args, start server |
| `src/main.tsx` | React entry point |
| `src/App.tsx` | Root component, state management |
| `src/types.ts` | Frontend type definitions |
| `src/index.css` | Global dark theme (based on mockup.html palette) |
| `src/hooks/useSSE.ts` | EventSource hook for real-time updates |
| `src/hooks/useTimeline.ts` | Fetch + cache timeline data |
| `src/components/TopBar.tsx` | Top status bar |
| `src/components/AgentSidebar.tsx` | Left sidebar with checkboxes |
| `src/components/AgentItem.tsx` | Single agent row |
| `src/components/TimelinePanel.tsx` | Centre panel: scrollable message list |
| `src/components/MessageBubble.tsx` | Single message with role label + content |
| `src/components/CollapsibleBlock.tsx` | Expandable tool output / debug-dump details |
| `src/components/InteractionArrow.tsx` | Cross-agent interaction indicator |
| `src/components/InfoPanel.tsx` | Right sidebar with stats |
| `src/components/TimeDivider.tsx` | Timestamp separator |

### New files in agent codebase

| File | Responsibility |
|------|---------------|
| `appfs-agent/rust/crates/rusty-claude-cli/src/debug_dump.rs` | Serialise ApiRequest to JSONL + write agent-meta.json |

### Modified files in agent codebase

| File | Change |
|------|--------|
| `appfs-agent/rust/crates/rusty-claude-cli/Cargo.toml` | Add `debug-dump` feature flag |
| `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs` | 4-line `cfg` call in `stream()` |

---

## Session JSONL Format Reference

Each session JSONL file contains lines with a `type` field. The dashboard only cares about two record types:

**`session_meta` record (first line):**
```json
{"type":"session_meta","version":1,"session_id":"session-abc123","created_at_ms":1747407780000,"updated_at_ms":1747407790000,"workspace_root":"/path/to/project","invoked_skills":[],"appfs_event_cursors":{},"appfs_wake_event_cursors":{}}
```

**`message` record (one per conversation turn):**
```json
{"type":"message","message":{"uuid":"msg-001","role":"user","blocks":[{"type":"text","text":"hello"}],"attachment_metadata":{"kind":"input_router"}}}
```
```json
{"type":"message","message":{"uuid":"msg-002","role":"assistant","blocks":[{"type":"text","text":"hi"},{"type":"tool_use","id":"tu-1","name":"read_file","input":"{\"path\":\"foo.rs\"}"}],"usage":{"input_tokens":3200,"output_tokens":18,"cache_creation_input_tokens":0,"cache_read_input_tokens":1200}}}
```
```json
{"type":"message","message":{"uuid":"msg-003","role":"tool","blocks":[{"type":"tool_result","tool_use_id":"tu-1","tool_name":"read_file","output":"fn main() {}","is_error":false}]}}
```

Content block types: `text`, `tool_use`, `tool_result`.
Roles: `system`, `user`, `assistant`, `tool`.
Optional fields: `usage`, `subtype`, `compact_metadata`, `attachment_metadata`, `hook_result_metadata`, `is_compact_summary`, `is_visible_in_transcript_only`.

---

## Task 1: Project Scaffolding

**Files:**
- Create: `dashboard/package.json`
- Create: `dashboard/index.html`
- Create: `dashboard/vite.config.ts`
- Create: `dashboard/tsconfig.json`
- Create: `dashboard/server/package.json`
- Create: `dashboard/server/tsconfig.json`

- [ ] **Step 1: Create root package.json**

```json
{
  "name": "appfs-debug-dashboard",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "concurrently \"npm run dev:server\" \"npm run dev:client\"",
    "dev:server": "npm --prefix server run dev",
    "dev:client": "vite",
    "build": "vite build",
    "start": "npm --prefix server start"
  },
  "devDependencies": {
    "concurrently": "^9.1.0",
    "vite": "^6.3.0",
    "@vitejs/plugin-react": "^4.4.0",
    "typescript": "^5.8.0"
  },
  "dependencies": {
    "react": "^19.1.0",
    "react-dom": "^19.1.0"
  }
}
```

- [ ] **Step 2: Create server package.json**

```json
{
  "name": "appfs-debug-dashboard-server",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "tsx watch src/index.ts",
    "start": "tsx src/index.ts"
  },
  "dependencies": {
    "fastify": "^5.3.0",
    "chokidar": "^4.0.0",
    "@fastify/cors": "^11.0.0"
  },
  "devDependencies": {
    "tsx": "^4.19.0",
    "typescript": "^5.8.0",
    "@types/node": "^22.15.0"
  }
}
```

- [ ] **Step 3: Create tsconfig files**

`dashboard/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "outDir": "dist"
  },
  "include": ["src"]
}
```

`dashboard/server/tsconfig.json`:
```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "outDir": "dist"
  },
  "include": ["src"]
}
```

- [ ] **Step 4: Create index.html**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>AppFS Debug Dashboard</title>
</head>
<body>
  <div id="root"></div>
  <script type="module" src="/src/main.tsx"></script>
</body>
</html>
```

- [ ] **Step 5: Create vite.config.ts**

```typescript
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': 'http://localhost:3100',
    },
  },
});
```

- [ ] **Step 6: Install dependencies**

Run:
```bash
cd dashboard && npm install && cd server && npm install
```

- [ ] **Step 7: Commit scaffolding**

```bash
git add dashboard/package.json dashboard/index.html dashboard/vite.config.ts dashboard/tsconfig.json dashboard/server/package.json dashboard/server/tsconfig.json
git commit -m "chore(dashboard): scaffold project with vite, fastify, react"
```

---

## Task 2: Server Types and JSONL Parser

**Files:**
- Create: `dashboard/server/src/types.ts`
- Create: `dashboard/server/src/jsonl-parser.ts`

These define the shared data model and the parser that reads session JSONL files. The types map 1:1 to the Rust structs in `session.rs`.

- [ ] **Step 1: Create server/src/types.ts**

```typescript
// ── Session JSONL record types ──

export interface SessionMetaRecord {
  type: 'session_meta';
  version: number;
  session_id: string;
  created_at_ms: number;
  updated_at_ms: number;
  workspace_root?: string;
  invoked_skills?: unknown[];
  appfs_event_cursors?: Record<string, unknown>;
  appfs_wake_event_cursors?: Record<string, unknown>;
}

export interface MessageRecord {
  type: 'message';
  message: ConversationMessage;
}

export type JsonlRecord = SessionMetaRecord | MessageRecord | { type: string };

// ── Conversation message model ──

export interface ConversationMessage {
  uuid: string;
  role: 'system' | 'user' | 'assistant' | 'tool';
  blocks: ContentBlock[];
  usage?: TokenUsage;
  subtype?: string;
  compact_metadata?: unknown;
  attachment_metadata?: AttachmentMetadata;
  hook_result_metadata?: unknown;
  is_compact_summary?: boolean;
  is_visible_in_transcript_only?: boolean;
}

export type ContentBlock =
  | { type: 'text'; text: string }
  | { type: 'tool_use'; id: string; name: string; input: string }
  | { type: 'tool_result'; tool_use_id: string; tool_name: string; output: string; is_error: boolean };

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
}

export interface AttachmentMetadata {
  kind: string;
}

// ── Debug-dump record (from agent debug-dump feature) ──

export interface DebugDumpRecord {
  type: 'message_request';
  timestamp_ms: number;
  request_index: number;
  model: string;
  max_tokens: number;
  system_prompt: string;
  system_prompt_length: number;
  message_count: number;
  messages: { role: string; content: string }[];
  tools_count: number;
  tools: { name: string; description: string }[];
  stream: boolean;
  reasoning_effort: string | null;
}

// ── Agent discovery ──

export interface AgentMeta {
  agent_name: string;
  principal_id: string;
  session_id: string;
  model: string;
  pid: number;
  started_at_ms: number;
  session_jsonl_path: string;
}

// ── Dashboard API types ──

export interface AgentInfo {
  name: string;
  principalId: string;
  sessionId: string;
  model: string;
  pid: number;
  startedAt: number;
  sessionJsonlPath: string;
  status: 'online' | 'offline';
  messageCount: number;
  totalInputTokens: number;
  totalOutputTokens: number;
}

export interface TimelineEntry {
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump';
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  raw: ConversationMessage | DebugDumpRecord;
  usage?: TokenUsage;
}

export interface CrossAgentInteraction {
  fromAgent: string;
  toAgent: string;
  eventType: 'message.sent' | 'message.received' | 'message.read';
  timestamp: number;
  seq?: number;
  label: string;
}

// ── SSE events ──

export type SSEEvent =
  | { event: 'agent-online'; data: AgentInfo }
  | { event: 'message'; data: TimelineEntry }
  | { event: 'debug-dump'; data: TimelineEntry }
  | { event: 'agent-offline'; data: { agent_name: string } };
```

- [ ] **Step 2: Create server/src/jsonl-parser.ts**

```typescript
import type { JsonlRecord, MessageRecord, SessionMetaRecord } from './types.js';

/**
 * Parse a complete JSONL file into records.
 * Skips blank lines and records with unknown `type`.
 */
export function parseJsonl(content: string): JsonlRecord[] {
  const records: JsonlRecord[] = [];
  for (const line of content.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const record = JSON.parse(trimmed);
      if (record && typeof record === 'object' && typeof record.type === 'string') {
        records.push(record);
      }
    } catch {
      // Skip malformed lines
    }
  }
  return records;
}

/**
 * Parse only message records from JSONL content.
 */
export function parseMessages(content: string): MessageRecord[] {
  return parseJsonl(content).filter(
    (r): r is MessageRecord => r.type === 'message',
  );
}

/**
 * Extract session_meta record if present.
 */
export function parseMeta(content: string): SessionMetaRecord | undefined {
  return parseJsonl(content).find(
    (r): r is SessionMetaRecord => r.type === 'session_meta',
  );
}

/**
 * Parse only new lines appended since the last known line count.
 * Returns parsed records from the new lines only.
 */
export function parseNewLines(content: string, previousLineCount: number): JsonlRecord[] {
  const lines = content.split('\n');
  const newLines = lines.slice(previousLineCount);
  return parseJsonl(newLines.join('\n'));
}
```

- [ ] **Step 3: Commit**

```bash
git add dashboard/server/src/types.ts dashboard/server/src/jsonl-parser.ts
git commit -m "feat(dashboard): add server types and JSONL parser"
```

---

## Task 3: Agent Registry and File Watcher

**Files:**
- Create: `dashboard/server/src/agent-registry.ts`
- Create: `dashboard/server/src/file-watcher.ts`

- [ ] **Step 1: Create server/src/agent-registry.ts**

The registry discovers agents by scanning a directory for `agent-meta.json` files (Phase 3, from debug-dump) or by scanning session directories directly (Phase 1).

```typescript
import fs from 'node:fs';
import path from 'node:path';
import type { AgentInfo, AgentMeta, MessageRecord, SessionMetaRecord, TokenUsage } from './types.js';
import { parseJsonl, parseMessages, parseMeta } from './jsonl-parser.js';

export class AgentRegistry {
  private agents = new Map<string, AgentInfo>();
  private messages = new Map<string, MessageRecord[]>();
  private dumpDir: string;

  constructor(dumpDir: string) {
    this.dumpDir = dumpDir;
  }

  /** Scan for agents. First tries agent-meta.json files, then falls back to
   *  scanning session JSONL files in the dump dir itself. */
  discover(): void {
    // Phase 3 path: agent-meta.json files exist in dump dir
    const metaFiles = fs.readdirSync(this.dumpDir).filter(f => f.startsWith('agent-meta'));

    if (metaFiles.length > 0) {
      for (const file of metaFiles) {
        const meta: AgentMeta = JSON.parse(
          fs.readFileSync(path.join(this.dumpDir, file), 'utf-8'),
        );
        this.registerFromMeta(meta);
      }
      return;
    }

    // Phase 1 fallback: treat each *.jsonl file in dump dir as a session
    const jsonlFiles = fs.readdirSync(this.dumpDir).filter(f => f.endsWith('.jsonl'));
    for (const file of jsonlFiles) {
      const fullPath = path.join(this.dumpDir, file);
      this.registerFromSessionFile(fullPath);
    }
  }

  private registerFromMeta(meta: AgentMeta): void {
    const name = meta.agent_name;
    let sessionContent = '';
    if (meta.session_jsonl_path && fs.existsSync(meta.session_jsonl_path)) {
      sessionContent = fs.readFileSync(meta.session_jsonl_path, 'utf-8');
    }
    const msgs = parseMessages(sessionContent);
    const sess = parseMeta(sessionContent);

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
    const name = sess?.session_id ?? path.basename(fullPath, '.jsonl');

    this.agents.set(name, {
      name,
      principalId: name,
      sessionId: sess?.session_id ?? name,
      model: 'unknown',
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
```

- [ ] **Step 2: Create server/src/file-watcher.ts**

```typescript
import chokidar from 'chokidar';
import type { AgentRegistry } from './agent-registry.js';
import { parseNewLines } from './jsonl-parser.js';
import type { MessageRecord } from './types.js';

export type FileChangeHandler = (agentName: string, newRecords: MessageRecord[]) => void;

export class FileWatcher {
  private watcher: chokidar.FSWatcher | null = null;
  private lineCounts = new Map<string, number>();

  constructor(private registry: AgentRegistry) {}

  start(onChange: FileChangeHandler): void {
    const paths = this.registry.getSessionPaths();
    if (paths.length === 0) return;

    // Track initial line counts
    for (const p of paths) {
      this.lineCounts.set(p, this.countLines(p));
    }

    this.watcher = chokidar.watch(paths, {
      ignoreInitial: true,
      persistent: true,
    });

    this.watcher.on('change', (filePath: string) => {
      const agentName = this.findAgentByPath(filePath);
      if (!agentName) return;

      const fs = await import('node:fs');
      const content = fs.default.readFileSync(filePath, 'utf-8');
      const prevLines = this.lineCounts.get(filePath) ?? 0;
      const newRecords = parseNewLines(content, prevLines).filter(
        (r): r is MessageRecord => r.type === 'message',
      );
      this.lineCounts.set(filePath, content.split('\n').length);

      if (newRecords.length > 0) {
        this.registry.reloadAgent(agentName);
        onChange(agentName, newRecords);
      }
    });

    // Also watch for new agent-meta.json files in the dump dir
    this.watcher.add(this.registry.dumpDirectory + '/agent-meta*');
  }

  async stop(): Promise<void> {
    if (this.watcher) {
      await this.watcher.close();
    }
  }

  private countLines(filePath: string): number {
    try {
      const fs = require('node:fs');
      const content = fs.readFileSync(filePath, 'utf-8');
      return content.split('\n').length;
    } catch {
      return 0;
    }
  }

  private findAgentByPath(filePath: string): string | undefined {
    for (const agent of this.registry.getAgents()) {
      if (agent.sessionJsonlPath === filePath) {
        return agent.name;
      }
    }
    return undefined;
  }
}
```

- [ ] **Step 3: Fix file-watcher top-level await (make synchronous)**

The `start()` method has `await import` inside a non-async handler. Fix it to use the already-imported `fs`:

```typescript
// Replace the change handler in FileWatcher.start():
import fs from 'node:fs';

// Inside start() method, change handler:
this.watcher.on('change', (filePath: string) => {
  const agentName = this.findAgentByPath(filePath);
  if (!agentName) return;

  const content = fs.readFileSync(filePath, 'utf-8');
  const prevLines = this.lineCounts.get(filePath) ?? 0;
  const newRecords = parseNewLines(content, prevLines).filter(
    (r): r is MessageRecord => r.type === 'message',
  );
  this.lineCounts.set(filePath, content.split('\n').length);

  if (newRecords.length > 0) {
    this.registry.reloadAgent(agentName);
    onChange(agentName, newRecords);
  }
});
```

- [ ] **Step 4: Commit**

```bash
git add dashboard/server/src/agent-registry.ts dashboard/server/src/file-watcher.ts
git commit -m "feat(dashboard): add agent registry and file watcher"
```

---

## Task 4: REST API Routes

**Files:**
- Create: `dashboard/server/src/routes/agents.ts`
- Create: `dashboard/server/src/routes/messages.ts`
- Create: `dashboard/server/src/routes/timeline.ts`
- Create: `dashboard/server/src/routes/events.ts`

- [ ] **Step 1: Create routes/agents.ts**

```typescript
import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';

export function registerAgentsRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/agents', async () => {
    return registry.getAgents();
  });
}
```

- [ ] **Step 2: Create routes/messages.ts**

```typescript
import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';

export function registerMessagesRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/agents/:name/messages', async (request) => {
    const { name } = request.params as { name: string };
    const messages = registry.getMessages(decodeURIComponent(name));
    return messages;
  });
}
```

- [ ] **Step 3: Create routes/timeline.ts**

```typescript
import type { FastifyInstance } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { TimelineEntry, ContentBlock, CrossAgentInteraction } from '../types.js';

export function registerTimelineRoute(app: FastifyInstance, registry: AgentRegistry): void {
  app.get('/api/timeline', async (request) => {
    const query = request.query as { agents?: string };
    const agentNames = (query.agents ?? '').split(',').filter(Boolean).map(decodeURIComponent);

    if (agentNames.length === 0) {
      return { entries: [], interactions: [] };
    }

    const entries: TimelineEntry[] = [];
    const interactions: CrossAgentInteraction[] = [];

    for (const name of agentNames) {
      const msgs = registry.getMessages(name);
      for (const rec of msgs) {
        const msg = rec.message;
        const content = extractTextContent(msg.blocks);
        const entry: TimelineEntry = {
          agentName: name,
          timestamp: 0, // Session JSONL has no per-message timestamp; use index-based ordering
          source: 'session',
          role: msg.role,
          content,
          raw: msg,
          usage: msg.usage,
        };
        entries.push(entry);

        // Detect cross-agent interactions from user messages
        if (msg.role === 'user') {
          const receivedMatch = content.match(/\[appfs_event\]\s+type=message\.received\s+from=(\S+)/);
          if (receivedMatch) {
            interactions.push({
              fromAgent: receivedMatch[1],
              toAgent: name,
              eventType: 'message.received',
              timestamp: 0,
              label: `${receivedMatch[1]} → ${name} (message.received)`,
            });
          }
          const readMatch = content.match(/type=message\.read\s+from=(\S+)/);
          if (readMatch) {
            interactions.push({
              fromAgent: readMatch[1],
              toAgent: name,
              eventType: 'message.read',
              timestamp: 0,
              label: `${readMatch[1]} → ${name} (message.read)`,
            });
          }
        }
      }
    }

    // Sort by array index (stable, preserves insertion order within each agent)
    // Assign numeric index as proxy for timestamp
    let idx = 0;
    for (const entry of entries) {
      entry.timestamp = idx++;
    }

    return { entries, interactions };
  });
}

function extractTextContent(blocks: ContentBlock[]): string {
  return blocks
    .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
    .map(b => b.text)
    .join('\n');
}
```

- [ ] **Step 4: Create routes/events.ts (SSE)**

```typescript
import type { FastifyInstance, FastifyReply } from 'fastify';
import type { AgentRegistry } from '../agent-registry.js';
import type { MessageRecord, TimelineEntry, ContentBlock } from '../types.js';
import { FileWatcher } from '../file-watcher.js';

const SSE_CLIENTS: Set<FastifyReply> = new Set();

function sseSend(reply: FastifyReply, event: string, data: unknown): void {
  reply.raw.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
}

export function registerEventsRoute(app: FastifyInstance, registry: AgentRegistry): void {
  const watcher = new FileWatcher(registry);

  watcher.start((agentName: string, newRecords: MessageRecord[]) => {
    for (const rec of newRecords) {
      const msg = rec.message;
      const entry: TimelineEntry = {
        agentName,
        timestamp: Date.now(),
        source: 'session',
        role: msg.role,
        content: extractTextContent(msg.blocks),
        raw: msg,
        usage: msg.usage,
      };
      for (const client of SSE_CLIENTS) {
        sseSend(client, 'message', entry);
      }
    }
  });

  app.get('/api/events', async (request, reply) => {
    reply.raw.writeHead(200, {
      'Content-Type': 'text/event-stream',
      'Cache-Control': 'no-cache',
      Connection: 'keep-alive',
    });
    reply.raw.write('\n');
    SSE_CLIENTS.add(reply);

    request.raw.on('close', () => {
      SSE_CLIENTS.delete(reply);
    });

    // Keep the response open
    await new Promise(() => {});
  });
}

function extractTextContent(blocks: ContentBlock[]): string {
  return blocks
    .filter((b): b is { type: 'text'; text: string } => b.type === 'text')
    .map(b => b.text)
    .join('\n');
}
```

- [ ] **Step 5: Commit**

```bash
git add dashboard/server/src/routes/
git commit -m "feat(dashboard): add REST API routes and SSE endpoint"
```

---

## Task 5: Server Entry Point

**Files:**
- Create: `dashboard/server/src/index.ts`

- [ ] **Step 1: Create server entry**

```typescript
import Fastify from 'fastify';
import cors from '@fastify/cors';
import path from 'node:path';
import { AgentRegistry } from './agent-registry.js';
import { registerAgentsRoute } from './routes/agents.js';
import { registerMessagesRoute } from './routes/messages.js';
import { registerTimelineRoute } from './routes/timeline.js';
import { registerEventsRoute } from './routes/events.js';

const PORT = parseInt(process.env.PORT ?? '3100', 10);
const HOST = process.env.HOST ?? '127.0.0.1';
const DUMP_DIR = process.argv[2] ?? process.env.APPFS_DEBUG_DUMP_DIR ?? '';

if (!DUMP_DIR) {
  console.error('Usage: tsx src/index.ts <dump-dir>');
  console.error('   or: set APPFS_DEBUG_DUMP_DIR=<path>');
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

  const app = Fastify({ logger: false });
  await app.register(cors, { origin: true });

  registerAgentsRoute(app, registry);
  registerMessagesRoute(app, registry);
  registerTimelineRoute(app, registry);
  registerEventsRoute(app, registry);

  await app.listen({ port: PORT, host: HOST });
  console.log(`Dashboard API listening on http://${HOST}:${PORT}`);
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
```

- [ ] **Step 2: Verify server starts**

Run:
```bash
cd dashboard/server && npx tsx src/index.ts /tmp/appfs-debug-dump
```
Expected: prints "Discovered 0 agent(s)" and "Dashboard API listening on http://127.0.0.1:3100" (the dir may be empty, that's fine).

- [ ] **Step 3: Commit**

```bash
git add dashboard/server/src/index.ts
git commit -m "feat(dashboard): add server entry point with CLI args"
```

---

## Task 6: Frontend Shell and Global CSS

**Files:**
- Create: `dashboard/src/main.tsx`
- Create: `dashboard/src/App.tsx`
- Create: `dashboard/src/types.ts`
- Create: `dashboard/src/index.css`

- [ ] **Step 1: Create src/main.tsx**

```tsx
import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './index.css';

const root = createRoot(document.getElementById('root')!);
root.render(<App />);
```

- [ ] **Step 2: Create src/types.ts**

```typescript
export interface AgentInfo {
  name: string;
  principalId: string;
  sessionId: string;
  model: string;
  pid: number;
  startedAt: number;
  sessionJsonlPath: string;
  status: 'online' | 'offline';
  messageCount: number;
  totalInputTokens: number;
  totalOutputTokens: number;
}

export interface ContentBlock =
  | { type: 'text'; text: string }
  | { type: 'tool_use'; id: string; name: string; input: string }
  | { type: 'tool_result'; tool_use_id: string; tool_name: string; output: string; is_error: boolean };

export interface ConversationMessage {
  uuid: string;
  role: 'system' | 'user' | 'assistant' | 'tool';
  blocks: ContentBlock[];
  usage?: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens: number;
    cache_read_input_tokens: number;
  };
  attachment_metadata?: { kind: string };
  is_compact_summary?: boolean;
}

export interface DebugDumpRecord {
  type: 'message_request';
  timestamp_ms: number;
  request_index: number;
  model: string;
  max_tokens: number;
  system_prompt: string;
  system_prompt_length: number;
  message_count: number;
  messages: { role: string; content: string }[];
  tools_count: number;
  tools: { name: string; description: string }[];
  stream: boolean;
  reasoning_effort: string | null;
}

export interface TimelineEntry {
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump';
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
  raw: ConversationMessage | DebugDumpRecord;
  usage?: {
    input_tokens: number;
    output_tokens: number;
    cache_creation_input_tokens: number;
    cache_read_input_tokens: number;
  };
}

export interface CrossAgentInteraction {
  fromAgent: string;
  toAgent: string;
  eventType: string;
  timestamp: number;
  label: string;
}

export interface TimelineResponse {
  entries: TimelineEntry[];
  interactions: CrossAgentInteraction[];
}

export const AGENT_COLORS = ['#58a6ff', '#3fb950', '#d2a8ff', '#d29922', '#39d2c0', '#f778ba'] as const;

export function getAgentColor(index: number): string {
  return AGENT_COLORS[index % AGENT_COLORS.length];
}
```

- [ ] **Step 3: Create src/index.css (dark theme from mockup)**

```css
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  font-family: 'SF Mono', 'Cascadia Code', 'Consolas', monospace;
  background: #0d1117;
  color: #c9d1d9;
  height: 100vh;
}
#root { height: 100vh; display: flex; flex-direction: column; }

/* Top bar */
.topbar {
  background: #161b22; border-bottom: 1px solid #30363d;
  padding: 8px 16px; display: flex; align-items: center; justify-content: space-between;
  flex-shrink: 0;
}
.topbar h1 { font-size: 14px; color: #58a6ff; }
.topbar .status { display: flex; gap: 12px; font-size: 12px; color: #8b949e; }

/* Main three-column layout */
.main-layout { display: flex; flex: 1; overflow: hidden; }

/* Left sidebar */
.sidebar {
  width: 240px; background: #161b22; border-right: 1px solid #30363d;
  display: flex; flex-direction: column; flex-shrink: 0;
}
.sidebar-header {
  padding: 12px; border-bottom: 1px solid #30363d;
  font-size: 11px; text-transform: uppercase; color: #8b949e; letter-spacing: 1px;
}
.select-hint { font-size: 10px; color: #484f58; padding: 6px 12px; border-bottom: 1px solid #21262d; font-style: italic; }
.agent-list { flex: 1; overflow-y: auto; }

.agent-item {
  padding: 10px 12px; border-bottom: 1px solid #21262d;
  cursor: pointer; display: flex; align-items: flex-start; gap: 8px;
  transition: background 0.15s;
}
.agent-item:hover { background: #1c2128; }
.agent-item.active { background: #1c2128; }
.agent-checkbox { margin-top: 2px; accent-color: #58a6ff; cursor: pointer; }
.agent-name { font-size: 13px; font-weight: 600; color: #e6edf3; margin-bottom: 2px; }
.agent-meta { font-size: 11px; color: #8b949e; }
.agent-model { color: #d2a8ff; font-size: 11px; }
.status-badge { display: inline-block; padding: 1px 6px; border-radius: 10px; font-size: 10px; font-weight: 600; }
.status-badge.online { background: #1b4332; color: #3fb950; }
.status-badge.offline { background: #21262d; color: #8b949e; }

/* Sidebar usage */
.sidebar-usage { padding: 12px; border-top: 1px solid #30363d; }
.usage-row { display: flex; justify-content: space-between; font-size: 12px; margin-bottom: 3px; }
.usage-label { color: #8b949e; }
.usage-value { color: #e6edf3; }
.usage-value.highlight { color: #58a6ff; }
.usage-bar { background: #21262d; border-radius: 3px; height: 4px; margin-top: 4px; overflow: hidden; }
.usage-fill { height: 100%; border-radius: 3px; }
.usage-fill.input { background: #58a6ff; }
.usage-fill.output { background: #3fb950; }

/* Centre timeline */
.timeline { flex: 1; overflow-y: auto; padding: 16px; display: flex; flex-direction: column; gap: 4px; }
.timeline-header { display: flex; align-items: flex-start; justify-content: space-between; margin-bottom: 12px; }
.timeline-header h2 { font-size: 15px; color: #e6edf3; }
.selected-tags { display: flex; gap: 6px; align-items: center; margin-top: 4px; }
.selected-tag { padding: 2px 8px; border-radius: 4px; font-size: 11px; font-weight: 600; }
.filters { display: flex; gap: 8px; }
.filter-btn {
  background: #21262d; border: 1px solid #30363d; color: #8b949e;
  padding: 4px 10px; border-radius: 4px; font-size: 11px; cursor: pointer; font-family: inherit;
}
.filter-btn.active { background: #1f6feb33; border-color: #58a6ff; color: #58a6ff; }

/* Messages */
.msg { padding: 8px 12px; border-radius: 6px; font-size: 13px; line-height: 1.5; position: relative; }
.msg-role { font-size: 11px; font-weight: 600; margin-bottom: 4px; }
.msg-time { font-size: 10px; color: #484f58; position: absolute; top: 8px; right: 10px; }
.msg-tokens { font-size: 10px; color: #484f58; margin-top: 4px; }
.msg-agent-tag { font-size: 10px; font-weight: 700; padding: 1px 6px; border-radius: 3px; margin-right: 6px; }

.msg.user { background: #1c2128; border-left: 3px solid #58a6ff; }
.msg.user .msg-role { color: #58a6ff; }
.msg.assistant { background: #161b22; border: 1px solid #30363d; }
.msg.assistant .msg-role { color: #3fb950; }
.msg.tool { background: #0d1117; border: 1px dashed #30363d; font-size: 12px; }
.msg.tool .msg-role { color: #8b949e; }
.msg.system { background: #1c2128; border-left: 3px solid #d29922; }
.msg.system .msg-role { color: #d29922; }
.msg.debug-dump { background: #1a1225; border: 1px solid #341a5e; }
.msg.debug-dump .msg-role { color: #bc8cff; }

/* Collapsible */
.collapsible-toggle { font-size: 11px; color: #58a6ff; cursor: pointer; margin-top: 4px; }
.collapsible-content {
  display: none; white-space: pre-wrap; font-size: 12px;
  max-height: 300px; overflow-y: auto; background: #0d1117;
  padding: 8px; border-radius: 4px; margin-top: 4px;
}
.collapsible-content.open { display: block; }

/* Interaction arrow */
.interaction-arrow {
  display: flex; align-items: center; gap: 4px;
  margin: 4px 0; padding: 0 12px; font-size: 10px; color: #484f58;
}
.arrow-line { flex: 1; height: 1px; background: linear-gradient(to right, #58a6ff44, #3fb95044); }
.arrow-label { padding: 1px 8px; background: #21262d; border-radius: 3px; white-space: nowrap; }

/* Right info panel */
.info-panel {
  width: 280px; background: #161b22; border-left: 1px solid #30363d;
  padding: 12px; overflow-y: auto; flex-shrink: 0;
}
.info-section { margin-bottom: 16px; }
.info-title { font-size: 11px; text-transform: uppercase; color: #8b949e; letter-spacing: 1px; margin-bottom: 6px; }
.info-row { display: flex; justify-content: space-between; font-size: 12px; margin-bottom: 3px; }

/* Time divider */
.time-divider {
  display: flex; align-items: center; gap: 8px;
  margin: 12px 0 8px; font-size: 11px; color: #484f58;
}
.time-divider::before, .time-divider::after { content: ''; flex: 1; height: 1px; background: #21262d; }
```

- [ ] **Step 4: Create initial App.tsx shell**

```tsx
import React, { useEffect, useState, useCallback } from 'react';
import type { AgentInfo, TimelineResponse } from './types';
import { TopBar } from './components/TopBar';
import { AgentSidebar } from './components/AgentSidebar';
import { TimelinePanel } from './components/TimelinePanel';
import { InfoPanel } from './components/InfoPanel';

export function App() {
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [selectedAgents, setSelectedAgents] = useState<Set<string>>(new Set());
  const [timeline, setTimeline] = useState<TimelineResponse>({ entries: [], interactions: [] });
  const [filter, setFilter] = useState<string>('all');

  useEffect(() => {
    fetch('/api/agents')
      .then(r => r.json())
      .then((data: AgentInfo[]) => {
        setAgents(data);
        if (data.length > 0) {
          setSelectedAgents(new Set([data[0].name]));
        }
      })
      .catch(() => {});
  }, []);

  const loadTimeline = useCallback((names: string[]) => {
    if (names.length === 0) {
      setTimeline({ entries: [], interactions: [] });
      return;
    }
    fetch(`/api/timeline?agents=${names.map(encodeURIComponent).join(',')}`)
      .then(r => r.json())
      .then((data: TimelineResponse) => setTimeline(data))
      .catch(() => {});
  }, []);

  useEffect(() => {
    loadTimeline(Array.from(selectedAgents));
  }, [selectedAgents, loadTimeline]);

  const toggleAgent = (name: string) => {
    setSelectedAgents(prev => {
      const next = new Set(prev);
      if (next.has(name)) {
        if (next.size > 1) next.delete(name);
      } else {
        next.add(name);
      }
      return next;
    });
  };

  const filtered = filter === 'all'
    ? timeline.entries
    : timeline.entries.filter(e => {
        if (filter === 'model') return e.role === 'assistant' || e.source === 'debug-dump';
        if (filter === 'tools') return e.role === 'tool' || e.content.includes('tool_use');
        if (filter === 'cross') return timeline.interactions.some(i => i.fromAgent === e.agentName || i.toAgent === e.agentName);
        return true;
      });

  return (
    <>
      <TopBar agentCount={agents.filter(a => a.status === 'online').length} />
      <div className="main-layout">
        <AgentSidebar
          agents={agents}
          selected={selectedAgents}
          onToggle={toggleAgent}
        />
        <TimelinePanel
          selectedAgents={Array.from(selectedAgents)}
          entries={filtered}
          interactions={timeline.interactions}
          filter={filter}
          onFilterChange={setFilter}
        />
        <InfoPanel
          agents={agents.filter(a => selectedAgents.has(a.name))}
          interactions={timeline.interactions}
        />
      </div>
    </>
  );
}
```

- [ ] **Step 5: Verify Vite dev server starts**

Run:
```bash
cd dashboard && npx vite --host 127.0.0.1 --port 5173
```
Expected: Vite starts, browser opens, shows a white page (components not yet created). Stop with Ctrl+C.

- [ ] **Step 6: Commit**

```bash
git add dashboard/src/
git commit -m "feat(dashboard): add React app shell, types, and global CSS"
```

---

## Task 7: TopBar and AgentSidebar Components

**Files:**
- Create: `dashboard/src/components/TopBar.tsx`
- Create: `dashboard/src/components/AgentSidebar.tsx`
- Create: `dashboard/src/components/AgentItem.tsx`

- [ ] **Step 1: Create TopBar.tsx**

```tsx
import React from 'react';

interface Props {
  agentCount: number;
}

export function TopBar({ agentCount }: Props) {
  return (
    <div className="topbar">
      <h1>AppFS Debug Dashboard</h1>
      <div className="status">
        <span style={{ color: agentCount > 0 ? '#3fb950' : '#484f58' }}>
          {agentCount} agent{agentCount !== 1 ? 's' : ''} online
        </span>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Create AgentItem.tsx**

```tsx
import React from 'react';
import type { AgentInfo } from '../types';
import { getAgentColor } from '../types';

interface Props {
  agent: AgentInfo;
  checked: boolean;
  colorIndex: number;
  onToggle: () => void;
}

export function AgentItem({ agent, checked, colorIndex, onToggle }: Props) {
  const color = getAgentColor(colorIndex);
  return (
    <div
      className={`agent-item ${checked ? 'active' : ''}`}
      style={{ borderLeft: checked ? `3px solid ${color}` : '3px solid transparent' }}
      onClick={onToggle}
    >
      <input
        type="checkbox"
        className="agent-checkbox"
        checked={checked}
        onChange={(e) => { e.stopPropagation(); onToggle(); }}
        onClick={(e) => e.stopPropagation()}
      />
      <div>
        <div className="agent-name">{agent.name}</div>
        <div className="agent-meta">principal: {agent.principalId}</div>
        <div className="agent-model">{agent.model}</div>
        <div style={{ marginTop: 4 }}>
          <span className={`status-badge ${agent.status}`}>{agent.status}</span>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Create AgentSidebar.tsx**

```tsx
import React from 'react';
import type { AgentInfo } from '../types';
import { AgentItem } from './AgentItem';

interface Props {
  agents: AgentInfo[];
  selected: Set<string>;
  onToggle: (name: string) => void;
}

export function AgentSidebar({ agents, selected, onToggle }: Props) {
  const totalInput = agents.reduce((s, a) => s + a.totalInputTokens, 0);
  const totalOutput = agents.reduce((s, a) => s + a.totalOutputTokens, 0);
  const maxTokens = Math.max(totalInput, totalOutput, 1);

  return (
    <div className="sidebar">
      <div className="sidebar-header">Agents (multi-select)</div>
      <div className="select-hint">Click to toggle. Select 2+ for merged timeline.</div>
      <div className="agent-list">
        {agents.map((agent, i) => (
          <AgentItem
            key={agent.name}
            agent={agent}
            checked={selected.has(agent.name)}
            colorIndex={i}
            onToggle={() => onToggle(agent.name)}
          />
        ))}
      </div>
      <div className="sidebar-usage">
        <div className="sidebar-header" style={{ padding: 0, border: 'none', marginBottom: 8 }}>Total Usage</div>
        <div className="usage-row">
          <span className="usage-label">Input</span>
          <span className="usage-value">{totalInput.toLocaleString()} tok</span>
        </div>
        <div className="usage-bar"><div className="usage-fill input" style={{ width: `${(totalInput / maxTokens) * 100}%` }} /></div>
        <div className="usage-row" style={{ marginTop: 6 }}>
          <span className="usage-label">Output</span>
          <span className="usage-value">{totalOutput.toLocaleString()} tok</span>
        </div>
        <div className="usage-bar"><div className="usage-fill output" style={{ width: `${(totalOutput / maxTokens) * 100}%` }} /></div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Commit**

```bash
git add dashboard/src/components/TopBar.tsx dashboard/src/components/AgentSidebar.tsx dashboard/src/components/AgentItem.tsx
git commit -m "feat(dashboard): add TopBar and AgentSidebar components"
```

---

## Task 8: Timeline and Message Components

**Files:**
- Create: `dashboard/src/components/TimelinePanel.tsx`
- Create: `dashboard/src/components/MessageBubble.tsx`
- Create: `dashboard/src/components/CollapsibleBlock.tsx`
- Create: `dashboard/src/components/InteractionArrow.tsx`
- Create: `dashboard/src/components/TimeDivider.tsx`

- [ ] **Step 1: Create CollapsibleBlock.tsx**

```tsx
import React, { useState } from 'react';

interface Props {
  label: string;
  children: React.ReactNode;
}

export function CollapsibleBlock({ label, children }: Props) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div className="collapsible-toggle" onClick={() => setOpen(!open)}>
        {open ? '▼' : '▶'} {label}
      </div>
      <div className={`collapsible-content ${open ? 'open' : ''}`}>{children}</div>
    </div>
  );
}
```

- [ ] **Step 2: Create MessageBubble.tsx**

```tsx
import React from 'react';
import type { TimelineEntry, ContentBlock, ConversationMessage } from '../types';
import { getAgentColor } from '../types';
import { CollapsibleBlock } from './CollapsibleBlock';

interface Props {
  entry: TimelineEntry;
  agentColorIndex: number;
}

export function MessageBubble({ entry, agentColorIndex }: Props) {
  const color = getAgentColor(agentColorIndex);
  const isDebugDump = entry.source === 'debug-dump';
  const roleClass = isDebugDump ? 'debug-dump' : entry.role;

  return (
    <div className={`msg ${roleClass}`}>
      <div className="msg-role">
        <span className="msg-agent-tag" style={{ background: `${color}33`, color }}>
          {entry.agentName}
        </span>
        {isDebugDump ? '[debug-dump] MessageRequest' : entry.role}
      </div>
      <div className="msg-content">
        {isDebugDump ? renderDebugDump(entry) : renderBlocks(entry)}
      </div>
      {entry.usage && (
        <div className="msg-tokens">
          input: {entry.usage.input_tokens.toLocaleString()} | output: {entry.usage.output_tokens.toLocaleString()}
          {entry.usage.cache_read_input_tokens > 0 && ` | cache_read: ${entry.usage.cache_read_input_tokens.toLocaleString()}`}
        </div>
      )}
    </div>
  );
}

function renderBlocks(entry: TimelineEntry): React.ReactNode {
  const msg = entry.raw as ConversationMessage;
  return msg.blocks.map((block, i) => {
    if (block.type === 'text') {
      return <div key={i}>{block.text}</div>;
    }
    if (block.type === 'tool_use') {
      return (
        <div key={i}>
          {block.name}({block.input.length > 200 ? block.input.slice(0, 200) + '...' : block.input})
        </div>
      );
    }
    if (block.type === 'tool_result') {
      return (
        <div key={i}>
          <CollapsibleBlock label={`Show output (${block.output.length.toLocaleString()} chars)`}>
            {block.output}
          </CollapsibleBlock>
        </div>
      );
    }
    return null;
  });
}

function renderDebugDump(entry: TimelineEntry): React.ReactNode {
  const dump = entry.raw as any; // DebugDumpRecord — will be typed in Phase 3
  return (
    <div>
      <div style={{ fontSize: 12, color: '#bc8cff', marginBottom: 4 }}>
        model: {dump.model ?? 'unknown'} | max_tokens: {dump.max_tokens ?? '?'}
      </div>
      {dump.system_prompt && (
        <CollapsibleBlock label={`Show system prompt (${dump.system_prompt_length ?? dump.system_prompt.length} chars)`}>
          {dump.system_prompt}
        </CollapsibleBlock>
      )}
      {dump.tools && dump.tools.length > 0 && (
        <CollapsibleBlock label={`Show tools (${dump.tools.length} definitions)`}>
          {JSON.stringify(dump.tools, null, 2)}
        </CollapsibleBlock>
      )}
    </div>
  );
}
```

- [ ] **Step 3: Create InteractionArrow.tsx**

```tsx
import React from 'react';
import type { CrossAgentInteraction } from '../types';

interface Props {
  interaction: CrossAgentInteraction;
}

export function InteractionArrow({ interaction }: Props) {
  return (
    <div className="interaction-arrow">
      <div className="arrow-line" />
      <div className="arrow-label">{interaction.label}</div>
      <div className="arrow-line" />
    </div>
  );
}
```

- [ ] **Step 4: Create TimeDivider.tsx**

```tsx
import React from 'react';

interface Props {
  label: string;
}

export function TimeDivider({ label }: Props) {
  return <div className="time-divider">{label}</div>;
}
```

- [ ] **Step 5: Create TimelinePanel.tsx**

```tsx
import React from 'react';
import type { TimelineEntry, CrossAgentInteraction, AgentInfo } from '../types';
import { getAgentColor } from '../types';
import { MessageBubble } from './MessageBubble';
import { InteractionArrow } from './InteractionArrow';

interface Props {
  selectedAgents: string[];
  entries: TimelineEntry[];
  interactions: CrossAgentInteraction[];
  filter: string;
  onFilterChange: (f: string) => void;
}

const FILTERS = [
  { key: 'all', label: 'All' },
  { key: 'model', label: 'Model I/O' },
  { key: 'tools', label: 'Tools' },
  { key: 'cross', label: 'Cross-agent' },
];

export function TimelinePanel({ selectedAgents, entries, interactions, filter, onFilterChange }: Props) {
  const agentColorMap = new Map(selectedAgents.map((name, i) => [name, i]));

  // Interleave interaction arrows between messages
  const rendered: React.ReactNode[] = [];
  let interactionIdx = 0;

  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    rendered.push(
      <MessageBubble
        key={`msg-${i}`}
        entry={entry}
        agentColorIndex={agentColorMap.get(entry.agentName) ?? 0}
      />,
    );

    // Insert interaction arrow if the next interaction's position is after this message
    while (interactionIdx < interactions.length) {
      const inter = interactions[interactionIdx];
      // Place interaction arrow after a message from the fromAgent
      if (inter.fromAgent === entry.agentName && i < entries.length - 1) {
        rendered.push(
          <InteractionArrow key={`arrow-${interactionIdx}`} interaction={inter} />
        );
        interactionIdx++;
      } else {
        break;
      }
    }
  }

  return (
    <div className="timeline">
      <div className="timeline-header">
        <div>
          <h2>{selectedAgents.length > 1 ? 'Merged Timeline' : `${selectedAgents[0] ?? 'No Agent'} Timeline`}</h2>
          {selectedAgents.length > 1 && (
            <div className="selected-tags">
              {selectedAgents.map((name, i) => {
                const color = getAgentColor(i);
                return (
                  <span key={name} className="selected-tag" style={{ background: `${color}33`, color }}>
                    {name}
                  </span>
                );
              })}
              <span style={{ color: '#484f58', fontSize: 11 }}>
                {selectedAgents.length} agents, {entries.length} messages
              </span>
            </div>
          )}
        </div>
        <div className="filters">
          {FILTERS.map(f => (
            <button
              key={f.key}
              className={`filter-btn ${filter === f.key ? 'active' : ''}`}
              onClick={() => onFilterChange(f.key)}
            >
              {f.label}
            </button>
          ))}
        </div>
      </div>
      {rendered}
    </div>
  );
}
```

- [ ] **Step 6: Commit**

```bash
git add dashboard/src/components/TimelinePanel.tsx dashboard/src/components/MessageBubble.tsx dashboard/src/components/CollapsibleBlock.tsx dashboard/src/components/InteractionArrow.tsx dashboard/src/components/TimeDivider.tsx
git commit -m "feat(dashboard): add timeline panel, message bubbles, and interaction arrows"
```

---

## Task 9: InfoPanel Component

**Files:**
- Create: `dashboard/src/components/InfoPanel.tsx`

- [ ] **Step 1: Create InfoPanel.tsx**

```tsx
import React from 'react';
import type { AgentInfo, CrossAgentInteraction } from '../types';

interface Props {
  agents: AgentInfo[];
  interactions: CrossAgentInteraction[];
}

export function InfoPanel({ agents, interactions }: Props) {
  if (agents.length === 0) {
    return <div className="info-panel"><div className="info-section"><div className="info-title">No agents selected</div></div></div>;
  }

  return (
    <div className="info-panel">
      {agents.length > 1 && (
        <div className="info-section">
          <div className="info-title">Merged View</div>
          <div className="info-row"><span className="info-label">Agents</span><span className="info-value highlight">{agents.length} selected</span></div>
          <div className="info-row"><span className="info-label">Interactions</span><span className="info-value">{interactions.length} cross-agent</span></div>
        </div>
      )}

      {agents.map(agent => (
        <div className="info-section" key={agent.name}>
          <div className="info-title">{agent.name}</div>
          <div className="info-row"><span className="info-label">Model</span><span className="info-value">{agent.model}</span></div>
          <div className="info-row"><span className="info-label">Messages</span><span className="info-value highlight">{agent.messageCount}</span></div>
          <div className="info-row"><span className="info-label">Input</span><span className="info-value">{agent.totalInputTokens.toLocaleString()} tok</span></div>
          <div className="info-row"><span className="info-label">Output</span><span className="info-value">{agent.totalOutputTokens.toLocaleString()} tok</span></div>
        </div>
      ))}

      {interactions.length > 0 && (
        <div className="info-section">
          <div className="info-title">Cross-Agent Events</div>
          <div style={{ background: '#0d1117', border: '1px solid #30363d', borderRadius: 4, padding: 8, fontSize: 11 }}>
            {interactions.map((inter, i) => (
              <div key={i} style={{ marginBottom: 4, color: '#8b949e' }}>
                <span style={{ color: '#3fb950' }}>{inter.fromAgent}</span>
                {' → '}
                <span style={{ color: '#58a6ff' }}>{inter.toAgent}</span>
                {' '}
                <span style={{ color: '#484f58' }}>({inter.eventType})</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Verify full frontend renders**

Run:
```bash
cd dashboard && npx vite --host 127.0.0.1 --port 5173
```
Open http://127.0.0.1:5173. Expected: three-panel dark layout renders with empty "No Agent Timeline" and "No agents selected". The API call fails silently since the server isn't running, but the UI should not crash.

- [ ] **Step 3: Commit**

```bash
git add dashboard/src/components/InfoPanel.tsx
git commit -m "feat(dashboard): add InfoPanel component"
```

---

## Task 10: SSE Real-Time Hook

**Files:**
- Create: `dashboard/src/hooks/useSSE.ts`

- [ ] **Step 1: Create useSSE.ts**

```tsx
import { useEffect, useRef, useCallback } from 'react';
import type { TimelineEntry } from '../types';

interface SSECallbacks {
  onMessage?: (entry: TimelineEntry) => void;
  onDebugDump?: (entry: TimelineEntry) => void;
  onAgentOnline?: (agent: unknown) => void;
  onAgentOffline?: (agent: unknown) => void;
}

export function useSSE(url: string, callbacks: SSECallbacks) {
  const callbacksRef = useRef(callbacks);
  callbacksRef.current = callbacks;

  useEffect(() => {
    const source = new EventSource(url);

    source.addEventListener('message', (e) => {
      try {
        const entry: TimelineEntry = JSON.parse(e.data);
        callbacksRef.current.onMessage?.(entry);
      } catch {}
    });

    source.addEventListener('debug-dump', (e) => {
      try {
        const entry: TimelineEntry = JSON.parse(e.data);
        callbacksRef.current.onDebugDump?.(entry);
      } catch {}
    });

    source.addEventListener('agent-online', (e) => {
      try {
        callbacksRef.current.onAgentOnline?.(JSON.parse(e.data));
      } catch {}
    });

    source.addEventListener('agent-offline', (e) => {
      try {
        callbacksRef.current.onAgentOffline?.(JSON.parse(e.data));
      } catch {}
    });

    return () => source.close();
  }, [url]);
}
```

- [ ] **Step 2: Wire SSE into App.tsx**

Add SSE hook usage to `dashboard/src/App.tsx`. Add the import and hook call after the existing `useEffect` blocks:

```tsx
// Add to imports:
import { useSSE } from './hooks/useSSE';

// Add inside App function, after the loadTimeline useEffect:
  useSSE('/api/events', {
    onMessage: (entry) => {
      setTimeline(prev => ({
        ...prev,
        entries: [...prev.entries, entry],
      }));
    },
    onDebugDump: (entry) => {
      setTimeline(prev => ({
        ...prev,
        entries: [...prev.entries, entry],
      }));
    },
    onAgentOnline: (agent: any) => {
      setAgents(prev => {
        if (prev.some(a => a.name === agent.name)) return prev;
        return [...prev, agent];
      });
    },
  });
```

- [ ] **Step 3: Commit**

```bash
git add dashboard/src/hooks/useSSE.ts dashboard/src/App.tsx
git commit -m "feat(dashboard): add SSE real-time hook and wire into App"
```

---

## Task 11: Agent Debug-Dump Rust Module

**Files:**
- Create: `appfs-agent/rust/crates/rusty-claude-cli/src/debug_dump.rs`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/Cargo.toml`
- Modify: `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`

- [ ] **Step 1: Add feature flag to Cargo.toml**

Find the `[features]` section in `appfs-agent/rust/crates/rusty-claude-cli/Cargo.toml` and add:

```toml
[features]
default = []
debug-dump = []
```

If `[features]` already exists, add `debug-dump = []` to it.

- [ ] **Step 2: Create debug_dump.rs**

```rust
//! Optional debug-dump module (gated by `debug-dump` feature).
//! Writes raw MessageRequest payloads to a JSONL file for the dashboard.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde_json::json;

/// Write agent metadata file on startup.
pub fn write_agent_meta(dir: &str, agent_name: &str, principal_id: &str, session_id: &str, model: &str, pid: u32, session_jsonl_path: &str) {
    let dir = Path::new(dir);
    let _ = fs::create_dir_all(dir);
    let meta = json!({
        "agent_name": agent_name,
        "principal_id": principal_id,
        "session_id": session_id,
        "model": model,
        "pid": pid,
        "started_at_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        "session_jsonl_path": session_jsonl_path,
    });
    let path = dir.join(format!("agent-meta-{agent_name}.json"));
    if let Ok(mut file) = File::create(&path) {
        let _ = writeln!(file, "{}", serde_json::to_string_pretty(&meta).unwrap_or_default());
    }
}

/// Append a single MessageRequest dump to the agent's JSONL file.
///
/// Uses runtime's `ContentBlock::to_json().render()` to serialise blocks
/// using the crate's own JSON type, then parses back into serde_json::Value
/// for the final output record.
pub fn write_request(dir: &str, session_id: &str, request: &crate::runtime::conversation::ApiRequest) {
    let dir = Path::new(dir);
    let _ = fs::create_dir_all(dir);
    let path = dir.join(format!("{session_id}.jsonl"));

    let system_prompt = request.system_prompt.join("\n\n");
    let messages: Vec<serde_json::Value> = request.messages.iter().map(|m| {
        let role_str = match m.role {
            crate::runtime::session::MessageRole::System => "system",
            crate::runtime::session::MessageRole::User => "user",
            crate::runtime::session::MessageRole::Assistant => "assistant",
            crate::runtime::session::MessageRole::Tool => "tool",
        };
        // ContentBlock::to_json() returns runtime's JsonValue.
        // .render() gives a valid JSON string we can parse into serde_json.
        let blocks_json: Vec<serde_json::Value> = m.blocks.iter().map(|b| {
            let rendered = crate::runtime::session::ContentBlock::to_json(b).render();
            serde_json::from_str(&rendered).unwrap_or(json!(rendered))
        }).collect();
        json!({
            "role": role_str,
            "blocks": blocks_json,
        })
    }).collect();

    let record = json!({
        "type": "message_request",
        "timestamp_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        "request_index": 0,
        "model": request.model_override.clone().unwrap_or_default(),
        "max_tokens": 0,
        "system_prompt": system_prompt,
        "system_prompt_length": system_prompt.len(),
        "message_count": messages.len(),
        "messages": messages,
        "tools_count": 0,
        "tools": [],
        "stream": true,
        "reasoning_effort": request.reasoning_effort,
    });

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{}", serde_json::to_string(&record).unwrap_or_default());
    }
}
```

Reference: `ApiRequest` is defined at `appfs-agent/rust/crates/runtime/src/conversation.rs:34`:
```rust
pub struct ApiRequest {
    pub system_prompt: Vec<String>,
    pub messages: Vec<ConversationMessage>,
    pub allow_tools: bool,
    pub model_override: Option<String>,
    pub reasoning_effort: Option<String>,
}
```
Import in `debug_dump.rs`: `use crate::runtime::conversation::ApiRequest;` (the runtime crate is a dependency of rusty-claude-cli).

- [ ] **Step 3: Add call site in main.rs**

In `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs`, at the top of `AnthropicRuntimeClient::stream()` (around line 8544), add:

```rust
    #[cfg(feature = "debug-dump")]
    if let Ok(dir) = std::env::var("APPFS_DEBUG_DUMP_DIR") {
        crate::debug_dump::write_request(&dir, &self.session_id, &request);
    }
```

Also add the module declaration near the top of `main.rs`:

```rust
#[cfg(feature = "debug-dump")]
mod debug_dump;
```

- [ ] **Step 4: Verify it compiles (without the feature)**

Run:
```bash
cd appfs-agent/rust && cargo check -p rusty-claude-cli
```
Expected: compiles successfully (feature is off by default, so the module is excluded).

- [ ] **Step 5: Verify it compiles (with the feature)**

Run:
```bash
cd appfs-agent/rust && cargo check -p rusty-claude-cli --features debug-dump
```
Expected: compiles. If `ApiRequest` field names don't match, adjust the `write_request` function accordingly.

- [ ] **Step 6: Commit**

```bash
git add appfs-agent/rust/crates/rusty-claude-cli/src/debug_dump.rs appfs-agent/rust/crates/rusty-claude-cli/Cargo.toml appfs-agent/rust/crates/rusty-claude-cli/src/main.rs
git commit -m "feat(agent): add debug-dump feature flag for dashboard"
```

---

## Task 12: End-to-End Smoke Test

This task validates the entire pipeline with a real JSONL file.

- [ ] **Step 1: Create a test session JSONL fixture**

Create `dashboard/test-fixtures/session-abc123.jsonl`:

```
{"type":"session_meta","version":1,"session_id":"session-abc123","created_at_ms":1747407778000,"updated_at_ms":1747407791000}
{"type":"message","message":{"uuid":"msg-001","role":"system","blocks":[{"type":"text","text":"You are an AI agent attached to an AppFS mount."}]}}
{"type":"message","message":{"uuid":"msg-002","role":"user","blocks":[{"type":"text","text":"[Pending input reminder]\n[1] [appfs_event] type=message.received\nFrom: code-implementer\n\"I've started the implementation.\""}]}}
{"type":"message","message":{"uuid":"msg-003","role":"assistant","blocks":[{"type":"text","text":"Acknowledged. I'll monitor the progress."}],"usage":{"input_tokens":2847,"output_tokens":22,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}
{"type":"message","message":{"uuid":"msg-004","role":"assistant","blocks":[{"type":"tool_use","id":"tu-1","name":"read_file","input":"{\"path\":\"tinode.rs\",\"offset\":2900}"}]}}
{"type":"message","message":{"uuid":"msg-005","role":"tool","blocks":[{"type":"tool_result","tool_use_id":"tu-1","tool_name":"read_file","output":"fn maybe_capture_read_receipt(&mut self, msg: &JsonValue) {\n    if let Some(info) = msg.get(\"info\") {\n        ...\n    }\n}","is_error":false}]}}
{"type":"message","message":{"uuid":"msg-006","role":"user","blocks":[{"type":"text","text":"[Pending input reminder]\n[1] [appfs_event] type=message.read from=code-implementer seq=47\n    (auto mark-as-read: sending {note})"}]}}
```

- [ ] **Step 2: Start the server against the fixture**

Run:
```bash
cd dashboard/server && npx tsx src/index.ts ../test-fixtures
```
Expected output:
```
Discovered 1 agent(s) in .../test-fixtures
  - session-abc123 (unknown, 6 messages)
Dashboard API listening on http://127.0.0.1:3100
```

- [ ] **Step 3: Verify API responses**

In a separate terminal:
```bash
curl http://127.0.0.1:3100/api/agents
```
Expected: JSON array with one agent entry.

```bash
curl http://127.0.0.1:3100/api/timeline?agents=session-abc123
```
Expected: JSON with `entries` (6 items) and `interactions` (1 item for message.read from code-implementer).

- [ ] **Step 4: Start the frontend and verify in browser**

Run:
```bash
cd dashboard && npx vite --host 127.0.0.1 --port 5173
```
Open http://127.0.0.1:5173. Expected:
- Left sidebar shows "session-abc123" agent
- Timeline shows 6 messages (system, user, assistant, tool_use, tool_result, user)
- Info panel shows message count, token usage
- "message.read" interaction arrow appears

- [ ] **Step 5: Commit fixture**

```bash
git add dashboard/test-fixtures/
git commit -m "test(dashboard): add session JSONL fixture for smoke test"
```

---

## Self-Review Checklist

**1. Spec coverage:**
- [x] Single-agent view: Tasks 1-9
- [x] Multi-agent merged timeline: Task 8 (TimelinePanel with selectedAgents)
- [x] SSE real-time: Task 10
- [x] Debug-dump feature: Task 11
- [x] Cross-agent interaction arrows: Task 8 (InteractionArrow)
- [x] Agent colour labels: Task 7-8
- [x] Info panel stats: Task 9

**2. Placeholder scan:** No TBD/TODO found. All steps contain actual code.

**3. Type consistency:**
- `AgentInfo` interface defined consistently in `server/src/types.ts` and `src/types.ts`
- `TimelineEntry`, `CrossAgentInteraction` match between server and frontend
- `ContentBlock` union type matches the Rust `ContentBlock` enum (text, tool_use, tool_result)
- `MessageRecord.message` → `ConversationMessage` mapping is correct
- `debug_dump.rs` references `crate::runtime::conversation::ApiRequest` — needs verification against actual struct location

One issue found: `debug_dump.rs` references `crate::runtime::conversation::ApiRequest` but the actual path may differ. The step notes that the engineer should verify the field names.
