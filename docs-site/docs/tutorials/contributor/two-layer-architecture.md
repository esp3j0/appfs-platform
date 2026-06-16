---
title: 理解两层架构
description: appfs（文件系统层）与 appfs-agent（Agent 层）如何通过挂载的应用树解耦协作。
sidebar_position: 1
---

# 理解两层架构

> **状态：脚手架占位** · 本页给出阅读地图，待补充图解与代码导引。

本教程结束时，你能说清：

- 哪些职责属于 `appfs`，哪些属于 `appfs-agent`
- 两者通过**挂载的应用树**与**事件流**解耦的具体边界
- 改动一个功能时，应该先动哪一层（以及 sync workflow）

## 两个层面各管什么

| appfs（文件系统层） | appfs-agent（Agent 层） |
|---|---|
| compose / 应用策略 | 附着 / 检测 AppFS |
| 运行时 supervisor | 系统提示 + skill 列表 |
| 挂载运行时 + 桥接 | 输入路由 / 事件提醒 |
| 挂载的应用树 | 交互式回合循环（REPL） |
| `_stream/events.evt.jsonl` | shell + 文件工具 |

## 关键边界

Agent 把挂载的应用树当**普通文件**读写。AppFS 处理写入 `*.act` 文件的动作，并向 `_stream/events.evt.jsonl` 发事件。Agent 的输入路由消费这些事件，作为待处理输入注入回合循环。

## Sync 工作流（重要）

独立仓库是组件内部事实来源。改动顺序：

```
claw-code-parity → 独立 appfs-agent → 本 monorepo 的 appfs-agent/
```

**绝不**把 `claw-code-parity` 直接同步到这里。集成专用改动（脚本、契约、跨项目文档）才直接落本仓库。

## 下一步

- 深入身份与租约模型 → [Principal 生命周期](./principal-lifecycle)
- 理解动作如何流动 → [动作管线](./action-pipeline)

---

**权威来源**：仓库根 `CLAUDE.md` 的“Architecture”一节；ADR 见 `docs/adr/0001-monorepo-layout.md`、`docs/adr/0002-repo-workflow.md`。
