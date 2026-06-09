# AppFS Principal Agent Lifecycle

## Purpose

The dashboard now owns a project-scoped lifecycle surface for AppFS principal agents:

- create principal
- delete principal
- start principal agent
- stop principal agent
- resume principal agent
- list principal status

This layer is intentionally separate from both AppFS runtime internals and the session-oriented process APIs.

## Control Boundary

AppFS remains the source of truth for principal records and materialized views:

```text
.appfs/_appfs/principals.registry.json
.appfs/_appfs/principals/status.res.json
.appfs/_appfs/principals/<principal-id>.res.json
```

The dashboard must not edit those JSON files directly. Principal mutations go through AppFS action files:

```text
.appfs/_appfs/principals/create_principal.act
.appfs/_appfs/principals/delete_principal.act
.appfs/_appfs/principals/update_principal.act
```

The dashboard server appends action lines and treats the materialized JSON files as eventually consistent runtime views.

## Server Surface

Canonical APIs are project-scoped:

```text
GET    /api/projects/:projectId/principals
POST   /api/projects/:projectId/principals
DELETE /api/projects/:projectId/principals/:principalId
POST   /api/projects/:projectId/principals/:principalId/start
POST   /api/projects/:projectId/principals/:principalId/stop
POST   /api/projects/:projectId/principals/:principalId/resume
```

The implementation lives in:

```text
dashboard/server/src/principal-lifecycle.ts
dashboard/server/src/routes/principals.ts
dashboard/server/src/process-manager.ts
```

The old session-oriented APIs remain valid for direct process/session operations:

```text
POST /api/process/spawn
POST /api/agents/:sessionId/stop
```

New UI and future agent/team tools should prefer the principal-scoped APIs.

## Future Agent Tool Integration

Future Claude Code-style `Agent` teammate mode should call the dashboard/server `PrincipalLifecycleService` or an equivalent AppFS launcher bridge.

It should not:

- write `ProjectRegistry` or `AgentProcessManager` state directly
- edit `principals.registry.json`
- duplicate principal create/resume/stop/delete semantics
- spawn multiple live agents for the same principal without an explicit takeover design

Recommended mapping:

```text
Agent(name = "coder")
  -> ensure/create principal "coder"
  -> start or resume principal agent "coder"
  -> communicate through Tinode/AppFS app channels
```

## Deferred Decisions

- True attach takeover across live agents.
- Force delete for online principals.
- Stopping external agents not managed by dashboard.
- Whether CLI-only appfs-agent should launch teammate agents through a local dashboard server or through a smaller launcher bridge.
- Whether successful action materialization should be polled before returning from create/delete.
