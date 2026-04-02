# ADR 0001: Keep AppFS And appfs-agent In One Monorepo

## Status

Accepted

## Context

`appfs` and `appfs-agent` are different systems with a clear runtime boundary:

- `appfs` provides the filesystem-native protocol, mount/runtime behavior, bridges, and SDKs
- `appfs-agent` provides the agent runtime that reads, writes, and acts through filesystem semantics

They are not the same product, but they now need coordinated evolution:

- AppFS surface design affects agent behavior
- agent runtime expectations affect AppFS integration shape
- cross-project changes need end-to-end validation

Keeping them in separate repositories adds friction for:

- protocol iteration
- integration testing
- coordinated CI
- docs and architecture updates

## Decision

Use a monorepo with sibling projects:

- `appfs/`
- `appfs-agent/`
- `integration/`

Do not collapse them into a single codebase or shared workspace root.

## Consequences

### Benefits

- easier joint development and review
- simpler end-to-end testing
- one place for shared architecture and integration docs
- one CI entrypoint for the combined stack

### Trade-offs

- larger repository checkout
- root CI must be curated carefully to avoid becoming too slow
- nested historical repo files such as subproject `.github/` directories remain as internal project artifacts

## Follow-up

- add end-to-end AppFS + agent integration tests under `integration/`
- define the first stable runtime contract between `appfs` and `appfs-agent`
- later decide whether release automation should also move to the monorepo root
