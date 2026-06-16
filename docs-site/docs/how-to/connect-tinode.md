---
title: 接入 Tinode 对话后端
description: 用 compose 把 Tinode 聊天后端接到 AppFS，并自动预热凭证。
---

# 接入 Tinode 对话后端

> **状态：脚手架占位**。

本指南解决：把 Tinode WebSocket 聊天后端接到 AppFS，让 Agent 能收发聊天消息。

## 环境变量

```bash
export APPFS_TINODE_ENDPOINT=<tinode-url>
export APPFS_TINODE_API_KEY=<api-key>
export APPFS_TINODE_LOGIN_PREFIX=<每次运行唯一前缀，避免凭证碰撞>
export APPFS_TINODE_CREDENTIAL_POLICY=auto-create   # 自动凭证预热
```

> ⚠️ Tinode 密钥与 API key **只**放环境变量或 runner secrets——绝不进 compose 文件、事件、skill 或会话日志。

## 步骤

1. 使用 `appfs/appfs-compose.tinode.local.yaml` 指向本地 Tinode。
2. Agent 在 warmup 阶段写 `ensure_credentials.act`，等 `profile.credentials.ready`。
3. 通过 `<app>/contacts/<id>/send_message.act` 发消息。

---

**权威来源**：compose 配置见 `appfs/appfs-compose.*.yaml`；Tinode connector 见 `appfs/sdk/rust/src/` 的 `tinode_connector.rs`；端到端脚本见 `integration/scripts/test-windows-appfs-tinode-multi-agent-smoke.ps1`。
