---
title: 项目贡献者路径
description: 深入 AppFS 平台内部——两层架构、生命周期、动作管线、多 Agent 编排。
sidebar_position: 0
---

# 项目贡献者路径

这条路径面向**想改这个平台的人**。走完你能说清：两层架构的边界、principal/attach 生命周期、`.act` 动作如何流动、多 Agent 如何共享上下文。

## 前置准备

- 能阅读 Rust 与 TypeScript 源码
- 先扫一遍仓库根 `CLAUDE.md` 的“Architecture”一节

## 学习顺序

1. [理解两层架构](./two-layer-architecture.md) —— appfs 与 appfs-agent 的边界
2. [Principal 生命周期](./principal-lifecycle.md) —— 身份与租约的七阶段
3. [动作管线](./action-pipeline.md) —— `.act` 如何被消费并发事件

> 只想用 AppFS、不参与开发？切到[应用开发者路径](../user/index.md)。
