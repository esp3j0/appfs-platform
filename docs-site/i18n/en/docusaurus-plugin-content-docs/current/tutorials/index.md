---
title: Tutorials
description: Pick the learning path that fits you — application developer or platform contributor.
---

# Tutorials

The AppFS platform combines two things: a **filesystem protocol and runtime for AI agents** (`appfs`), and an **interactive agent runtime that attaches to it** (`appfs-agent`). This section contains **hands-on tutorials** — linear, reproducible, "follow along and it runs." It does not explain *why* (that's [Explanation](/docs/explanation)) and does not list APIs (that's [Reference](/docs/reference)).

## Two learning paths

Choose the one that matches your goal.

### 🧑‍💻 Application developer path

You want to **use** AppFS as the state layer for your own AI agent app: mount the filesystem, attach an agent, read/write app state, send and receive events.

**Start with:** _Mount AppFS and attach an agent_ → `tutorials/user/mount-and-attach`

### 🔧 Platform contributor path

You want to **modify** the platform: understand the two-layer architecture, the principal/attach lifecycle, the `.act` action pipeline, and multi-agent orchestration.

**Start with:** _Understand the two-layer architecture_ → `tutorials/contributor/two-layer-architecture`

> Translating the rest of this section into English is the next incremental step. The Chinese originals under `docs/` are authoritative.

## Prerequisites

- **Platform:** Windows (WinFsp) / Linux (FUSE) / macOS (NFS) — mount support differs per platform.
- **Runtimes:** Node.js ≥ 20, Rust toolchain (to build `agentfs` / `claw`), Python (SDKs + agent Python layer).
- **Real model calls:** `ANTHROPIC_API_KEY` for tutorials that drive a live agent.
