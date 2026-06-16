---
title: Principal 生命周期
description: create / attach / warmup / heartbeat / sweep / detach / delete 七阶段跨三层流转。
sidebar_position: 2
---

# Principal 生命周期

> **状态：脚手架占位** · 阶段图已就位，待补充每阶段的代码入口与事件。

**principal** 是 Agent 在 AppFS 中的身份；**attach lease** 是绑定到该 principal 的一个活跃进程。生命周期横跨三层（dashboard / appfs / appfs-agent），是多 Agent 运行的核心。

## 七个阶段

1. **Create** —— 追加 `create_principal.act`；supervisor 在 `principals.registry.json` 注册，materialize 私有应用，发 `principal.created`。
2. **Attach** —— Agent 写 `attach_principal.act`；创建 `AppfsAttachLease`。规则：一个 principal 可有多个 attach id，但**同时只能有一个非过期 attach**（除非 `takeover`）。
3. **Warmup** —— 私有应用：写 `ensure_credentials.act`，等 `profile.credentials.ready` 才进回合循环。
4. **Heartbeat** —— headless Agent 每 **30s** 写 `update_principal.act`（无 `agent_status` → 仅刷新 `last_seen_at`）。
5. **Sweep** —— supervisor 每 **30s** 执行 `sweep_stale_attaches_once()`；**90s** 未见的 attach 被丢弃，塌缩为 Offline。
6. **Stop / Detach** —— `detach_principal.act`（best-effort）。
7. **Delete / Archive** —— `delete_principal.act`（残留过期 attach 时带 `force: true`）；会话归档并从 resume 过滤掉。

> 状态更新：带 `agent_status` 的 `update_principal.act` 设置 `agent_status.state`。`APPFS_MULTI_AGENT_MODE_SHARED` 控制是否多 Agent 共享同一 principal 上下文。

## 下一步

- 理解动作如何被消费与分发 → [动作管线](./action-pipeline)
- 在 dashboard 上手动驱动这套流程 → [实操：多 Agent](../../how-to/configure-multi-agent)

---

**权威来源**：完整设计见 `docs/agent-lifecycle-architecture.md`；PRD 见 `docs/APPFS-principal-agent-lifecycle.md`；dashboard 侧实现见 `dashboard/server/src/principal-lifecycle.ts`。
