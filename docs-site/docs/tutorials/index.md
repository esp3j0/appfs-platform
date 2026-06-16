---
title: 教程总览
description: 选择适合你的学习路径——应用开发者或项目贡献者。
sidebar_position: 0
---

# 教程总览

AppFS 平台把两件事拼在一起：一个**面向 AI Agent 的文件系统协议与运行时**（`appfs`），以及一个**附着到该文件系统的交互式 Agent 运行时**（`appfs-agent`）。本节是**手把手教程**——线性、可复现、照着做就能跑通，不解释为什么（那是[原理](../explanation)的事），也不罗列 API（那是[参考](../reference)的事）。

## 两条学习路径

请根据你的目标选择一条：

### 🧑‍💻 应用开发者路径

你想**用** AppFS 给自己的 AI Agent 应用做状态层：挂载文件系统、附着 Agent、读写应用状态、收发事件。

→ 从 [挂载 AppFS 并附着 Agent](./user/mount-and-attach) 开始 · [查看路径总览](./user)

### 🔧 项目贡献者路径

你想**改**这个平台：理解两层架构、principal/attach 生命周期、`.act` 动作管线、多 Agent 编排。

→ 从 [理解两层架构](./contributor/two-layer-architecture) 开始 · [查看路径总览](./contributor)

## 前置准备

- **平台**：Windows（WinFsp）/ Linux（FUSE）/ macOS（NFS）—— 挂载能力随平台不同
- **运行时**：Node.js ≥ 20（文档站本身），Rust toolchain（构建 `agentfs` / `claw`），Python（SDK 与 agent Python 层）
- **真实模型调用**：`ANTHROPIC_API_KEY`（教程中涉及真实 Agent 对话时）

> 如果你只想先看懂项目全貌、不急着跑代码，建议直接读 [两层分离的动机](../explanation/two-layer-separation)。
