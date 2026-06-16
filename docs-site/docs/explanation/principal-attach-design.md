---
title: Principal / Attach 租约的设计权衡
description: 为什么一个 principal 只能有一个非过期 attach，以及 takeover / sweep 的取舍。
---

# Principal / Attach 租约的设计权衡

> **状态：脚手架占位** · 待补充为什么是 90s / 30s 等具体阈值的依据。

## 身份 vs. 会话

刻意把两件事拆开：

- **principal** = 身份（长期、跨重启）
- **attach lease** = 会话（短期、绑定进程）

这让一个 Agent 身份可以被多个进程在不同时间附着，而不会丢失历史。

## 为什么同时只允许一个非过期 attach

避免两个进程争抢同一 principal 上下文导致状态撕裂。规则：

- 一个 principal 可有多个 attach id，但**同时只有一个非过期 attach**。
- 新 attach 自动 takeover 过期的；新鲜的冲突会被拒。
- 例外：`takeover: true` 显式强制接管。

## 为什么用心跳 + sweep 而非锁

分布式锁在崩溃恢复下会卡死。改用**心跳（30s）+ 容忍 sweep（90s = 3 次漏跳）**：崩溃的进程自然停止心跳，90s 后被 sweep 回收，无需人工干预。代价是最坏 90s 的陈旧窗口——对 Agent 场景可接受。

## 为什么 `force: true` 删除

删除带过期 attach 的 principal 时，残留 lease 会阻挡清理。`force: true` 让 delete 能强拆——但仅在确认 attach 已死时用。

---

**权威来源**：完整设计见 `docs/agent-lifecycle-architecture.md`；恢复策略见 `appfs-agent/rust/crates/runtime/src/recovery_recipes.rs`；principal 恢复与 warmup 稳定化见近期提交 `4ff60e77`。
