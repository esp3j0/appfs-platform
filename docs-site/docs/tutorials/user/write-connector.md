---
title: 写一个 Connector
description: 为你的应用实现一个 AppFS connector，把外部系统映射到挂载的应用树。
sidebar_position: 3
---

# 写一个 Connector

> **状态：脚手架占位** · 步骤骨架已就位。

本教程结束时，你将拥有一个把外部系统（HTTP/gRPC/Tinode）映射到挂载应用树的 connector。

## 选择连接器类型

- **HTTP bridge** —— 最常见，参考 `appfs/examples/appfs/bridges/http-python/`
- **gRPC bridge** —— 强类型，参考 `appfs/examples/appfs/bridges/grpc-python/`
- **Tinode** —— 聊天场景，参考 `appfs_connector.rs` 中的 tinode 实现

## 实现动作处理

Connector 监听 app 树下的 `*.act` 动作文件，处理后回写 `.res.json` 结果视图。契约见 [动作契约](../../reference/action-contract)。

## 发布到 compose

在 `appfs/appfs-compose.*.yaml` 中声明你的应用（参考 huoyan/aiim 配置）。

## 下一步

- 跑一次完整 HTTP 桥接端到端 → 见 `integration/scripts/test-windows-appfs-agent-http-demo.ps1`（IC-1）

---

**权威来源**：connector 架构见 `appfs/docs/v4/APPFS-v0.4-Connector结构接口.zh-CN.md`；快速上手见 `appfs/examples/appfs/ADAPTER-QUICKSTART.zh-CN.md`。
