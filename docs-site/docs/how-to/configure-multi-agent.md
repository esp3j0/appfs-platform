---
title: 配置多 Agent 共享 principal
description: 让多个 Agent 进程共享同一 principal 上下文，并避免 attach 租约冲突。
---

# 配置多 Agent 共享 principal

> **状态：脚手架占位**。

本指南解决：如何让多个 headless Agent 安全共享同一 principal，而不互相抢 attach 租约。

## 前提

- 已理解 [Principal 生命周期](../tutorials/contributor/principal-lifecycle)
- 每个并发 attach 的 `APPFS_ATTACH_ID` 必须互不相同（dashboard 用 `dashboard-<principal>`）

## 步骤

1. 设置 `APPFS_MULTI_AGENT_MODE_SHARED` 启用共享上下文。
2. 为每个 Agent 进程分配独立的 `APPFS_ATTACH_ID`。
3. 依赖 supervisor 的 takeover / sweep 规则管理租约（90s 超时）。
4. 通过共享任务队列协调多 Agent —— 见 `task_board.rs` / `worker_boot.rs`。

---

**权威来源**：多 Agent 身份与可见性设计见 `docs/APPFS-multi-agent-identity-and-app-visibility-v0-design.md`；流程验收见 `docs/APPFS-multi-agent-identity-流程验收文档.md`；冒烟脚本见 `integration/scripts/test-windows-appfs-tinode-multi-agent-smoke.ps1`（Tinode v0）。
