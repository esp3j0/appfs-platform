---
title: 事件流（events.evt.jsonl）
description: _stream/events.evt.jsonl 的 append-only 事件流与游标读取。
---

# 事件流（events.evt.jsonl）

> **状态：脚手架占位** · 事件类型表已就位，payload schema 待补充。

## 路径

```
/_stream/events.evt.jsonl     # append-only 事件流
/_stream/cursor.res.json      # Agent 侧游标
```

Agent 通过游标读取事件流，把事件作为待处理输入注入回合循环。

## 事件类型（由 supervisor 发出）

| 类型 | 触发 |
|---|---|
| `principal.created` | create_principal 成功 |
| `principal.*` | principal 状态变化 |
| `action.completed` | 动作处理成功 |
| `action.failed` | 动作处理失败 |
| `message` | 应用层消息 |

## 输入路由分类

`input_router.rs` 把输入分四类：`UserTerminal` / `AppfsEvent` / `AgentMessage` / `System`。每条输入是一个 `InputEnvelope`，可选携带 `principal_id` / `app_id` / `stream_id` / `seq`。投递方式：`InjectAtNextBoundary`（打断当前回合）或 `QueueAfterTurn`。

---

**权威来源**：消费侧见 `appfs-agent/rust/crates/runtime/src/appfs.rs` 与 `input_router.rs`；事件渲染设计见 `docs/plans/2026-05-19-appfs-event-model-rendering.md`。
