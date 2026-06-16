---
title: 教程总览
description: AppFS 是什么、解决什么问题、核心一圈，以及从哪开始。
sidebar_position: 0
---

# 教程总览

本节是**手把手教程**——线性、可复现、照着做就能跑通。它不解释“为什么”（那是[原理](../explanation)），也不罗列 API（那是[参考](../reference)）。先花两分钟了解这个项目。

## AppFS 是什么

AppFS 是一个**面向 AI Agent 的文件系统**。你把它挂载成一个普通目录，你的应用就以**文件和目录**的形式出现在里面；Agent 像读写普通文件一样读写它们，AppFS 在背后调用你应用的真实后端，并把结果变成事件。

它由两块组成：

- **`appfs`** —— 文件系统本身，外加一个运行时进程。它把外部应用桥接成挂载点里的文件，处理写入的动作，发出事件。
- **`appfs-agent`**（命令叫 `claw`）—— 一个 Agent 运行时。它附着到挂载点，读应用状态、写动作、对事件做出反应——就像一个能操作你应用的 AI 助手。

## 它解决什么问题

每接一个应用就给 Agent 写一套专门的 API，成本高，还容易和 Agent 已有的工具体系打架。AppFS 的思路是：**把“和应用的交互”统一成“读写文件”**——Agent 本来就擅长用文件。你的应用只要提供一个小型 HTTP 服务（叫 **connector**），声明“我长什么样、这是我的状态、这是你能对我做的动作”，剩下的交给 AppFS。

## 核心一圈

不管什么应用，套路都一样（后面的教程就是带你走这一圈）：

```
注册应用                    →  挂载点里长出应用的文件树
读 *.res.json / *.res.jsonl →  看到应用当前状态（快照）
写 *.act                    →  对应用做一个动作
读 _stream/events.evt.jsonl →  看到动作的结果（事件）
```

## 前置准备

- **Rust toolchain**（`cargo --version`）—— 构建 `agentfs` 与 `claw`
- **Python 3 + [uv](https://docs.astral.sh/uv/)** —— 跑示例 connector
- **平台挂载后端**：Windows 装 [WinFsp](https://winfsp.dev/)；Linux 装 `fuse3`；macOS 自带 NFS
- **`ANTHROPIC_API_KEY`** —— 仅“让 Agent 真实对话”的步骤需要；入门教程**不需要**

## 两条学习路径

选一条开始：

### 🧑‍💻 应用开发者路径

你想**用** AppFS 给自己的应用接上 Agent：挂载、注册应用、发消息，最后写出自己的 connector。

→ [挂载 AppFS 并附着 Agent](./user/mount-and-attach.mdx) · [路径总览](./user/index.md)

### 🔧 项目贡献者路径

你想**改**这个平台：两层架构、principal/attach 生命周期、`.act` 动作管线、多 Agent 编排。

→ [理解两层架构](./contributor/two-layer-architecture.md) · [路径总览](./contributor/index.md)

> 只想先看懂“为什么这样设计”、不急着跑代码？读 [两层分离的动机](../explanation/two-layer-separation.md)。
