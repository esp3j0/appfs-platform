---
title: 运行时清单（runtime.json）
description: supervisor 在 .well-known/appfs/runtime.json 发布的发现清单。
---

# 运行时清单（runtime.json）

> **状态：脚手架占位**。

supervisor 通过 `runtime_manifest.rs` 在挂载点发布清单，Agent 用它发现 AppFS。

## 发现路径

```
.well-known/appfs/runtime.json
```

Agent 的 AppFS 环境检测优先级：

1. `APPFS_MOUNT_ROOT`（显式挂载根，最高优先）
2. `runtime.json` manifest
3. CWD 向上的 `.appfs/` 启发式探测

也可以用 `APPFS_ATTACH_SCHEMA` 直接告诉 Agent 如何发现挂载。

---

**权威来源**：发布逻辑见 `appfs/cli/src/cmd/appfs/runtime_manifest.rs`；环境检测见 `appfs-agent/rust/crates/runtime/src/appfs.rs`；CLAUDE.md“Common Environment Variables”一节。
