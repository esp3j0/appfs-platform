# AppFS Debug Dashboard Design

## Problem

Multi-agent AppFS sessions are hard to debug. When `default` and `code-implementer` exchange messages through the Tinode bridge, there is no way to see what each agent actually sent to its LLM, how events were routed, or where a message got lost. The only option today is adding `eprintln!` to the Rust code, running tests, then removing the logging.

## Goal

Build a standalone HTTP dashboard that visualises the complete model I/O of every agent in an AppFS session, with minimal code impact on the agent runtime.

### Success Criteria

- Select 1 agent: see its full conversation timeline (system prompt, user/assistant messages, tool calls, debug-dump of raw MessageRequest).
- Select 2+ agents: see a merged timeline sorted by timestamp, with colour-coded agent labels and cross-agent interaction arrows showing message flow between agents.
- Real-time: new messages appear within ~1 second via SSE push.
- Agent code impact: one `debug-dump` feature flag adding ~15 lines to `main.rs`.
- Zero runtime overhead when the dashboard is not running (feature off = no file I/O).

## Architecture

```
Agent process (existing)                Dashboard (new)
========================               ========================

AnthropicRuntimeClient::stream()       +-----------+
  │                                    |  server/  |  Node.js + Fastify
  ├─ [debug-dump] write to             |  src/     |  - file watcher (chokidar)
  │    <dump-dir>/<session-id>.jsonl   |           |  - JSONL parser
  │                                    |           |  - SSE endpoint
  Session::push_message()              |           |  - REST API
  │                                    +-----+-----+
  ├─ write to                                |
  │   <session-dir>/session.jsonl            | HTTP + SSE
                                             |
                                     +------+------+
                                     |  src/        |  React + Vite
                                     |  components/ |  - AgentSidebar
                                     |              |  - MergedTimeline
                                     |              |  - MessageBubble
                                     |              |  - InfoPanel
                                     +--------------+
```

The dashboard is **fully independent**: it reads files the agent already writes (session JSONL) plus new debug-dump files (gated by feature flag). It does not import any agent code, share a database, or require a running agent to browse historical data.

## Data Sources

### Source 1: Session JSONL (existing, zero code change)

Each agent writes a `session.jsonl` file via `Session::push_message()`. One JSON line per `ConversationMessage`, containing:

- `role`: system / user / assistant
- `blocks[]`: text, tool_use, tool_result
- `usage`: input_tokens, output_tokens, cache_read, cache_write
- `timestamp_ms`: when the message was appended

**What it provides**: full conversation history per agent.

**What it lacks**: raw `MessageRequest` (assembled system prompt, tools schema, model name, reasoning_effort). Session JSONL records the assistant's perspective after message conversion, not the exact JSON sent to the LLM.

### Source 2: Debug-Dump JSONL (new, ~15 lines behind feature flag)

When the `debug-dump` Rust feature is enabled and `APPFS_DEBUG_DUMP_DIR` env is set, `AnthropicRuntimeClient::stream()` appends one JSON line per API call to `<dir>/<session-id>.jsonl`:

```json
{
  "type": "message_request",
  "timestamp_ms": 1747407781000,
  "request_index": 1,
  "model": "claude-sonnet-4-6",
  "max_tokens": 16384,
  "system_prompt_length": 2847,
  "system_prompt": "...",
  "message_count": 3,
  "messages": [
    {"role": "user", "content": "..."},
    {"role": "assistant", "content": "..."},
    {"role": "user", "content": "..."}
  ],
  "tools_count": 12,
  "tools": [{"name": "read_file", "description": "..."}],
  "stream": true,
  "reasoning_effort": null
}
```

This captures exactly what the LLM received. The feature is off by default (`cfg(feature = "debug-dump")`), so production builds have zero overhead.

### Agent-Side Changes

Single injection point in `appfs-agent/rust/crates/rusty-claude-cli/src/main.rs` at `AnthropicRuntimeClient::stream()`:

```rust
fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
    #[cfg(feature = "debug-dump")]
    if let Ok(dir) = std::env::var("APPFS_DEBUG_DUMP_DIR") {
        let _ = crate::debug_dump::write_request(&dir, &self.session_id, &request);
    }
    // ... existing code unchanged
}
```

Plus a small `debug_dump.rs` helper module (~30 lines) that serialises `ApiRequest` to JSONL.

**Total agent impact**: 1 new file (`debug_dump.rs`, ~30 lines), 1 `Cargo.toml` feature declaration, 4-line call site. No changes to runtime, API, or telemetry crates.

### Source Discovery

The server needs to find session files across multiple agent processes. Strategy:

1. `APPFS_DEBUG_DUMP_DIR` env (primary) — the debug-dump directory set at agent launch.
2. Inside the dump dir, each agent writes a small `agent-meta.json` at startup:

```json
{
  "agent_name": "code-implementer",
  "principal_id": "implementer",
  "session_id": "session-1747...a3f2",
  "model": "claude-sonnet-4-6",
  "pid": 48132,
  "started_at_ms": 1747407780000,
  "session_jsonl_path": "/home/user/.local/share/opencode/<hash>/session.jsonl"
}
```

3. The server reads all `agent-meta.json` files on startup and watches the directory for new ones (agents joining mid-session).

4. The `agent-meta.json` is written by the same `debug_dump` module, as the very first action when the dump dir is configured. If `debug-dump` is off, no meta file is written, and the dashboard falls back to scanning known session directories (best-effort).

## Backend Design

### Stack

- **Runtime**: Node.js 20+
- **Framework**: Fastify (lighter than Express, built-in schema validation)
- **File watching**: chokidar
- **SSE**: fastify-sse (or manual `text/event-stream` response)

### API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/agents` | GET | List all discovered agents with metadata |
| `/api/agents/:name/messages` | GET | Messages for a single agent (paginated) |
| `/api/agents/:name/debug-dumps` | GET | Debug-dump entries for a single agent |
| `/api/timeline?agents=a,b` | GET | Merged timeline for selected agents, sorted by timestamp |
| `/api/events` | GET (SSE) | Real-time push: `agent-online`, `message`, `debug-dump`, `agent-offline` |

### SSE Event Types

```
event: agent-online
data: {"agent_name": "code-implementer", "model": "claude-sonnet-4-6", ...}

event: message
data: {"agent_name": "default", "role": "assistant", "blocks": [...], "timestamp_ms": ..., "usage": {...}}

event: debug-dump
data: {"agent_name": "default", "request_index": 2, "model": "...", ...}

event: agent-offline
data: {"agent_name": "code-implementer"}
```

### File Watching

On startup:
1. Read `APPFS_DEBUG_DUMP_DIR` env or accept `--dump-dir` CLI flag.
2. Scan for `agent-meta.json` files, build in-memory agent registry.
3. For each agent, parse existing session JSONL and debug-dump JSONL into memory.
4. Start chokidar watchers on the dump dir and each session JSONL path.
5. On file change: parse new lines, push SSE events to connected clients.

### In-Memory Model

```typescript
interface AgentInfo {
  name: string;
  principalId: string;
  sessionId: string;
  model: string;
  pid: number;
  startedAt: number;
  sessionJsonlPath: string;
  status: 'online' | 'offline';
}

interface TimelineEntry {
  agentName: string;
  timestamp: number;
  source: 'session' | 'debug-dump';
  role: 'system' | 'user' | 'assistant' | 'tool-result';
  content: string;              // text summary
  raw: ConversationMessage | DebugDumpEntry;  // full JSON
  usage?: UsageStats;
}

interface CrossAgentInteraction {
  fromAgent: string;
  toAgent: string;
  eventType: 'message.sent' | 'message.received' | 'message.read';
  timestamp: number;
  seq?: number;
}
```

Cross-agent interactions are detected by correlating:
- User messages containing `[appfs_event] type=message.received from=<agent>` with the named agent's assistant messages.
- User messages containing `type=message.read from=<agent>` with read receipt data.
- These are pattern-matched from message content strings (Phase 1); Phase 2 can add structured metadata.

## Frontend Design

### Stack

- **Build**: Vite
- **UI**: React 19 + TypeScript
- **Styling**: CSS modules (no framework dependency, keeps bundle small)
- **Real-time**: native EventSource API for SSE

### Component Tree

```
App
├── TopBar                    // mount path, agent count, last update time
├── MainLayout (flex row)
│   ├── AgentSidebar          // checkbox list, per-agent colour, status badge
│   │   └── AgentItem[]       // name, principal, model, running/idle badge
│   ├── TimelinePanel         // centre: message list
│   │   ├── TimelineHeader    // selected agent tags, filter buttons
│   │   ├── TimeDivider[]     // timestamp separators
│   │   ├── MessageBubble[]   // agent-coloured label + role + content
│   │   │   └── CollapsibleBlock  // expandable tool output / debug-dump details
│   │   └── InteractionArrow[]    // cross-agent event indicators
│   └── InfoPanel             // right sidebar
│       ├── MergedViewStats   // agent count, message count, interaction count
│       ├── PerAgentStats[]   // per-agent model, usage, cost
│       └── CrossAgentEvents  // direction + event type list
```

### Interaction Flow

1. On load: `GET /api/agents` to populate sidebar.
2. Default: select first agent, show its timeline.
3. User checks additional agents: `GET /api/timeline?agents=default,code-implementer`, render merged view.
4. SSE connection: `EventSource('/api/events')` — push new messages and debug-dumps into the displayed timeline.
5. Filter buttons: client-side filter on `source` and `role` fields.

### Agent Colour Assignment

Static palette, assigned by order of discovery:

| Index | Colour | Hex |
|-------|--------|-----|
| 0 | Blue | `#58a6ff` |
| 1 | Green | `#3fb950` |
| 2 | Purple | `#d2a8ff` |
| 3 | Orange | `#d29922` |
| 4 | Cyan | `#39d2c0` |
| 5 | Pink | `#f778ba` |

Cycles if more than 6 agents.

## File Structure

```
dashboard/
├── mockup.html                    # static mockup (already exists)
├── package.json                   # root: scripts to run server + dev frontend
├── server/
│   ├── package.json
│   ├── tsconfig.json
│   └── src/
│       ├── index.ts               # Fastify entry, CLI args, start server
│       ├── agent-registry.ts      # discover and track agents from meta files
│       ├── jsonl-parser.ts        # streaming JSONL line parser
│       ├── file-watcher.ts        # chokidar wrapper, emit parsed records
│       ├── routes/
│       │   ├── agents.ts          # GET /api/agents
│       │   ├── messages.ts        # GET /api/agents/:name/messages
│       │   ├── timeline.ts        # GET /api/timeline?agents=a,b
│       │   └── events.ts          # GET /api/events (SSE)
│       └── types.ts               # shared TypeScript interfaces
├── src/                           # React frontend
│   ├── App.tsx
│   ├── main.tsx
│   ├── index.css                  # global dark theme
│   ├── types.ts
│   ├── hooks/
│   │   ├── useSSE.ts              # EventSource hook
│   │   └── useTimeline.ts         # fetch + cache timeline data
│   └── components/
│       ├── TopBar.tsx
│       ├── AgentSidebar.tsx
│       ├── AgentItem.tsx
│       ├── TimelinePanel.tsx
│       ├── MessageBubble.tsx
│       ├── CollapsibleBlock.tsx
│       ├── InteractionArrow.tsx
│       ├── InfoPanel.tsx
│       └── TimeDivider.tsx
├── vite.config.ts
├── tsconfig.json
└── index.html                     # Vite entry point
```

## Phasing

### Phase 1 — Single-agent view (MVP)

- Server: read session JSONL files, expose REST API.
- Frontend: agent sidebar + single-agent timeline + info panel.
- No agent code changes required.

### Phase 2 — Multi-agent merged timeline

- Server: merged timeline endpoint, cross-agent interaction detection.
- Frontend: multi-select, merged view, interaction arrows.
- SSE real-time push.

### Phase 3 — Debug-dump integration

- Agent: add `debug-dump` feature flag + `debug_dump.rs` module.
- Server: watch and parse debug-dump JSONL files.
- Frontend: `[debug-dump]` message blocks with collapsible system prompt and tools schema.

### Phase 4 — Polish (optional, later)

- Search/filter messages by content.
- Export timeline as JSON.
- Cost tracking with per-model pricing.
- Dark/light theme toggle.

## Open Questions

None. All key decisions resolved through brainstorming:
- Backend: Node.js + TypeScript + Fastify
- Frontend: React + Vite
- Data: session JSONL + debug-dump (both)
- Layout: left sidebar + centre timeline + right info panel
- Multi-select: merged timeline with agent colour labels and interaction arrows
