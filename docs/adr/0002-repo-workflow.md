# ADR 0002: Use Standalone Repos As Component Sources Of Truth

## Status

Accepted

## Context

The project currently has four relevant repositories:

- standalone `appfs`
- standalone `appfs-agent`
- monorepo `appfs-platform`
- external upstream `claw-code-parity`

Without a clear sync policy, the same logical component can drift across multiple places:

- upstream `claw-code-parity`
- standalone `appfs-agent`
- `appfs-platform/appfs-agent`

That creates ambiguity about:

- where component changes should be authored
- how upstream updates should enter the system
- whether monorepo edits should be pushed back into component repos

## Decision

Use the following ownership model:

- standalone `appfs` is the source of truth for AppFS component code
- standalone `appfs-agent` is the source of truth for agent component code
- `appfs-platform` is the source of truth for integration assets, combined CI, and cross-project documentation
- `claw-code-parity` feeds only into standalone `appfs-agent`

Use the following sync flow:

1. `claw-code-parity` -> standalone `appfs-agent`
2. standalone `appfs-agent` -> `appfs-platform/appfs-agent`
3. standalone `appfs` -> `appfs-platform/appfs`

Do not sync `claw-code-parity` directly into `appfs-platform`.

## Consequences

### Benefits

- each component has one obvious primary home
- upstream parity work is isolated to one repo
- monorepo history stays focused on integration and curated component syncs
- combined CI can validate the integrated stack without becoming the authoring surface for every component change

### Trade-offs

- developers must perform an explicit sync step into `appfs-platform`
- a quick fix made inside `appfs-platform/appfs` or `appfs-platform/appfs-agent` must be backported deliberately
- some work may briefly exist twice during backport and resync

## Operational Rules

For normal development:

- make `appfs` changes in the standalone `appfs` repo
- make `appfs-agent` changes in the standalone `appfs-agent` repo
- sync them into `appfs-platform` with subtree pull scripts

For integration-first work:

- it is acceptable to prototype in `appfs-platform`
- if the change belongs to `appfs` or `appfs-agent`, backport it to the standalone repo before further component syncs
- after the standalone repo lands the change, resync the subtree so the monorepo returns to a clean ownership model

## Remote Naming

Recommended local remote names:

- `appfs-platform`: `origin`, `appfs-repo`, `appfs-agent-repo`
- standalone `appfs-agent`: `origin`, `upstream`, `platform`
- standalone `appfs`: `origin`, `platform`

Extra personal remotes may exist, but scripts and docs should rely on the standard names above.
