---
title: 应用开发者路径
description: 用 AppFS 把你的应用接上 Agent——挂载、注册、发消息、写自己的 connector。
sidebar_position: 0
---

# 应用开发者路径

这条路径面向**想用 AppFS 的人**。走完你能：挂载文件系统、把应用接进来、读写应用状态、发动作、收事件，最后写出自己的 connector。

## 前置准备

- Rust toolchain、Python + uv
- 平台挂载后端（Windows 用 [WinFsp](https://winfsp.dev/)，Linux 用 fuse3，macOS 用 NFS）
- 入门的前两篇**不需要** `ANTHROPIC_API_KEY`；涉及真实 Agent 对话时才需要
- 完整清单见[教程总览](../)

## 学习顺序

1. [挂载 AppFS 并附着 Agent](./mount-and-attach.mdx) —— 把文件系统跑起来
2. [Agent 的第一条消息](./first-message.mdx) —— 走通应用消息往返
3. [写一个 Connector](./write-connector.mdx) —— 接你自己的应用

> 想反过来理解平台内部、参与开发？切到[项目贡献者路径](../contributor/index.md)。
