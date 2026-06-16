---
title: Reference
description: Authoritative field-level specs for the protocol and contracts — for lookup, not reading.
---

# Reference

This is **lookup-oriented** technical reference: the action contract, the runtime manifest, and the event-stream format. It does not explain *why* (that's [Explanation](/docs/explanation)) and does not walk you through anything (that's [Tutorials](/docs/tutorials)).

## Contents

- _Action contract (.act / .res.json)_ → `reference/action-contract`
- _Runtime manifest (runtime.json)_ → `reference/runtime-manifest`
- _Event stream (events.evt.jsonl)_ → `reference/event-stream`

## Conventions

- Every `.act` file is an **append-only action log**.
- The supervisor owns the `.res.json` result views and `principals.registry.json`.
- The agent reads `_stream/events.evt.jsonl` via a cursor.
