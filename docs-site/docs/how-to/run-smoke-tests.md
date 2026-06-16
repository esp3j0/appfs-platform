---
title: 运行集成冒烟测试
description: 用 integration/scripts 下的脚本端到端验证 AppFS + appfs-agent。
---

# 运行集成冒烟测试

> **状态：脚手架占位**。

本指南解决：如何跑通 `integration/scripts/` 下的端到端冒烟测试。这些脚本同时驱动 AppFS 与 appfs-agent，需要本地基础设施（Windows 需 WinFsp，Linux/macOS 需 FUSE/NFS）。

## 检查点速查

| 检查点 | 脚本 | 覆盖范围 |
|---|---|---|
| IC-0 | `test-windows-appfs-agent-smoke.ps1` | 基本挂载 + Agent 附着 + 状态 |
| IC-1 | `test-windows-appfs-agent-http-demo.ps1` | HTTP 桥 + 真实 Agent 提示 + 动作往返 |
| IC-2 | `test-windows-appfs-agent-multi-attach.ps1` | 同一挂载两个 Agent，独立 attach id |
| IC-3 | `test-windows-appfs-agent-launcher.ps1` | 联合 AppFS + Agent 启动 |
| Tinode v0 | `test-windows-appfs-tinode-multi-agent-smoke.ps1` | 多 principal、私有应用、凭证预热、跨 principal 消息 |
| IC-0 (Unix) | `test-unix-appfs-agent-smoke.sh` | Linux FUSE / macOS NFS 基本冒烟 |

## 运行

```bash
# 需要 ANTHROPIC_API_KEY 才能做真实模型调用
powershell -File integration/scripts/test-windows-appfs-agent-smoke.ps1
```

---

**权威来源**：完整脚本清单与说明见仓库根 `CLAUDE.md` 的“Integration Smoke Tests”表。
