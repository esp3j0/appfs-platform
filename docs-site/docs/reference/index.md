---
title: 参考手册
description: 协议与契约的权威字段说明——面向查阅，不面向阅读。
---

# 参考手册（Reference）

这里是**面向查阅**的技术参考：动作契约、运行时清单、事件流格式。不解释为什么（那是[原理](../explanation)），也不带你做（那是[教程](../tutorials)）。

## 内容

- [动作契约（.act / .res.json）](./action-contract)
- [运行时清单（runtime.json）](./runtime-manifest)
- [事件流（events.evt.jsonl）](./event-stream)

## 约定

- 所有 `.act` 文件都是 **append-only 动作日志**。
- supervisor 拥有 `.res.json` 结果视图与 `principals.registry.json` 注册表。
- Agent 侧通过游标读取 `_stream/events.evt.jsonl`。
