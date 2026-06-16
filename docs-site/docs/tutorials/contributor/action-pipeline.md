---
title: 动作管线
description: .act 动作如何被 action_consumer 消费、dispatch 到 handler、回写结果并发事件。
sidebar_position: 3
---

# 动作管线

> **状态：脚手架占位** · 数据流骨架已就位，待补充关键函数定位与示例事件。

本教程结束时，你能追踪一条 `.act` 动作从写入到事件发出的完整路径。

## 数据流

```
*.act (append-only)  →  action_consumer.rs (watch)
                    →  action_dispatcher.rs (route)
                    →  runtime_supervisor.rs (handle_*)
                    →  写回 *.res.json 结果视图
                    →  向 _stream/events.evt.jsonl 发事件
                       (principal.* / action.completed / action.failed / message)
```

## 关键路径

- **消费**：`appfs/cli/src/cmd/appfs/action_consumer.rs`
- **分发**：`appfs/cli/src/cmd/appfs/action_dispatcher.rs`
- **处理**：`runtime_supervisor.rs` 中的 `handle_*` 系列函数
- **清单发布**：`runtime_manifest.rs`（写 `runtime.json`）
- **注册表**：`registry.rs` / `registry_manager.rs`

应用专属动作住在应用树下，例如 `<app>/contacts/<id>/send_message.act`。

## 下一步

- 查阅所有动作与结果视图的字段 → [动作契约](../../reference/action-contract)
- 看事件如何被 Agent 消费 → [事件流](../../reference/event-stream)

---

**权威来源**：挂载契约见仓库根 `CLAUDE.md` 的“AppFS Mount Contract”；集成层动作契约见 `integration/APPFS-appfs-agent-attach-contract-v1.1.md`。
