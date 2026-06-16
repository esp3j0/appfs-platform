---
title: 挂载 AppFS 并附着 Agent
description: 从零挂载一个 AppFS 文件系统，注册一个 principal，并让一个 Agent 附着上去。
sidebar_position: 1
---

# 挂载 AppFS 并附着 Agent

> **状态：脚手架占位** · 本教程的步骤骨架已就位，具体命令待按你本地环境填充验证。

本教程结束时，你将拥有：

- 一个运行中的 AppFS 挂载点
- 一个已注册的 principal（Agent 身份）
- 一个附着到该 principal 的运行中 Agent 进程

## 第 1 步：构建 `agentfs` CLI

```bash
cd appfs/cli
cargo build --release
# 产物：target/release/agentfs
```

## 第 2 步：初始化并挂载

```bash
agentfs init my-app
agentfs mount my-app /mnt/appfs   # Linux FUSE；Windows 用 WinFsp，macOS 用 NFS
```

## 第 3 步：注册 principal

向 `/_appfs/principals/create_principal.act` 追加一条 JSON 动作（详见 [动作契约](../../reference/action-contract)）。

## 第 4 步：启动 Agent 并附着

```bash
APPFS_PRINCIPAL_ID=<你的 principal id> \
APPFS_ATTACH_ID=local-attach-01 \
claw
```

## 下一步

- 跑通第一条 Agent 消息 → [Agent 的第一条消息](./first-message)
- 给自己的应用写一个 connector → [写一个 Connector](./write-connector)

---

**权威来源**：命令清单见 `appfs/MANUAL.md`；挂载契约见仓库根 `CLAUDE.md` 的“AppFS Mount Contract”一节；端到端冒烟脚本见 `integration/scripts/test-windows-appfs-agent-smoke.ps1`（IC-0）。
