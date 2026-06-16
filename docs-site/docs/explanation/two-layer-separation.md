---
title: 两层分离的动机
description: 为什么把文件系统协议层与 Agent 运行时层拆开，通过挂载的应用树解耦。
---

# 两层分离的动机

> **状态：脚手架占位** · 论点骨架已就位，待补充权衡细节与历史。

## 为什么要拆成两层

AppFS（文件系统层）与 appfs-agent（Agent 层）通过**挂载的应用树**与**事件流**解耦。这样设计的关键收益：

- **Agent 不感知 AppFS 内部** —— 它把挂载的应用树当普通文件读写。这让 Agent 运行时可独立演进、甚至替换。
- **协议层可被多种 Agent 复用** —— 任何能读写文件、读 JSONL 游标的进程都能成为 Agent。
- **故障域隔离** —— supervisor 的动作管线与 Agent 的回合循环互不阻塞。

## 边界划在哪

关键边界是：**Agent 写 `*.act`，AppFS 发 `_stream/events.evt.jsonl`**。一切跨层交互都走这两个通道。这把"应用语义"（AppFS 处理动作）与"对话语义"（Agent 跑回合）彻底分开。

## 代价

- 调试时要同时看两层（dashboard 的 k-way 时间线合并正是为此而生）。
- Sync 工作流要求组件内部改动先落独立仓库，再 subtree 同步——多一步但保证单一事实来源。

---

**权威来源**：架构总览见仓库根 `CLAUDE.md`；monorepo 布局决策见 `docs/adr/0001-monorepo-layout.md`；方向讨论见 `docs/APPFS-AGENT-方向讨论-v0.2.md`。
