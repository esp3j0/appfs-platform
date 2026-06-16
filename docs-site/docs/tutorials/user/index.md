---
title: 应用开发者路径
description: 用 AppFS 给你的 AI Agent 应用做状态层——挂载、附着、读写、收发事件。
sidebar_position: 0
---

# 应用开发者路径

这条路径面向**想用 AppFS 的人**。走完后你能：挂载文件系统、注册并附着 Agent、用任一 SDK 读写应用状态、触发 Agent 回合并消费事件。

## 学习顺序

1. [挂载 AppFS 并附着 Agent](./mount-and-attach)
2. [Agent 的第一条消息](./first-message)
3. [写一个 Connector](./write-connector)

## 前置准备

见[教程总览](../)的前置准备。最少需要：能构建 `agentfs` CLI 的 Rust 环境，以及（涉及真实对话时）`ANTHROPIC_API_KEY`。

> 想反过来理解平台内部、参与开发？切到[项目贡献者路径](../contributor)。
