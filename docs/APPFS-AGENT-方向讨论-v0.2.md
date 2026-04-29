# AppFS ↔ appfs-agent 协作方向讨论

> 2026-04-26 · v0.2 · 基于 Claude Code TS 版 skill 发现机制修订

---

## 1. 这次修订的核心结论

`claude-code` 的 TS 版本并不是把所有 skill 的全文，或者 `when_to_use` 字段，直接塞进 system prompt。

它的做法更像三层：

1. **System prompt 只给薄引导**
   - 告诉模型有 `SkillTool`
   - 告诉模型 `/skill-name` 是 skill shorthand
   - 告诉模型“需要时必须先调用 skill”

2. **Skill 元数据通过 system-reminder 注入**
   - skill 名
   - description
   - when_to_use
   - 这些以 “可用 skills 列表” 的形式进入对话上下文

3. **Skill 全文只在真正调用时按需加载**
   - 用户显式 `/commit`
   - 或模型判断当前请求命中了某个 skill
   - 或 agent frontmatter 显式 preload skill

这意味着我们如果要做 AppFS ↔ appfs-agent 的深度协作，最值得参考的不是“把 AIIM 的全部说明全文注入 system prompt”，而是：

- AppFS 环境摘要进入 system prompt
- 当前 App 的 skill 摘要进入 system-reminder / model-facing listing
- 详细能力说明在真正需要时再加载

---

## 2. 当前 Rust 版和 TS 版的差距

### 2.1 Rust 版已经有的能力

- 已能发现 AppFS 环境：
  - `/.well-known/appfs/runtime.json`
  - `/_appfs/apps.registry.json`
  - 当前 app / 当前 mount 信息
- 已能解析 skill frontmatter：
  - `when_to_use`
  - `allowed-tools`
  - `paths`
- 已支持基于文件路径的 conditional skill activation

### 2.2 Rust 版还没有的关键一层

当前 Rust 版缺的，不是 skill 文档本身，而是 **model-facing discoverability**：

- 没有把当前可用 skill 列表注入上下文
- 没有把 `description + when_to_use` 暴露给模型做自动匹配
- 没有 “当前 App skill 摘要” 这层中介

所以今天的状态更像：

- `/skills aiim` 可以显式调用
- `paths:` 可以在访问某些文件后激活 skill
- 但用户只是自然语言说：
  - “给张三说一下明天开会”
  - 模型还不会像 TS 版那样，先看到一份当前 App skill listing，再主动决定调用对应 skill

---

## 3. 修订后的总体方向

### 3.1 目标体验

当 `appfs-agent` 运行在挂载了 `aiim` 的 AppFS 上时，用户可以直接说：

> 给张三说一下明天开会

理想链路是：

1. agent 先知道自己在 AppFS 环境中
2. agent 知道当前 app 是 `aiim`
3. agent 看到 `aiim` 的 skill/控制摘要
4. agent 知道：
   - 张三对应 `contacts/zhangsan/profile.res.json`
   - 发消息要写 `contacts/zhangsan/send_message.act`
   - payload schema 至少包含 `text`
5. agent 通过 bash / file 操作正确 append act
6. agent 观察 `_stream/events.evt.jsonl` 确认结果

### 3.2 实现原则

- 不把完整 App skill 全文默认塞进 system prompt
- 先注入 **简短的 AppFS 环境摘要**
- 再注入 **当前 app 的 skill listing / action listing**
- 真正需要时再加载当前 app 的详细 skill
- 事件流只默认跟平台控制面 + 当前 app，不默认订阅所有 app

---

## 4. 方案收敛：两条并行线

### 4.1 appfs-agent 侧：参考 TS 版的 skill 发现模型

#### Layer A: System prompt 注入 AppFS 环境摘要

建议内容控制在 150~300 tokens：

- 当前运行在 AppFS mount 中
- mount root
- 当前 app
- 平台控制面路径
- `.act` / `.res.jsonl` / `.evt.jsonl` 的语义
- 如果需要更详细 app 操作说明，请查看当前 app skill / 当前 app 控制说明

**注意**：这里只放环境摘要，不放 AIIM 的完整操作手册。

#### Layer B: 当前 app skill listing 注入

参考 TS 版 `skill_listing` 的做法，把当前 app 暴露成一个模型可发现的“摘要能力项”：

- skill name: `appfs-aiim`
- description: `AIIM incident chat and contact messaging`
- when_to_use:
  - 当用户提到联系人沟通、群聊消息、会议提醒、事故通知时使用
  - 当用户需要切换 AIIM scope 或刷新结构时使用

这层 listing 应该是：

- 面向模型的
- 摘要级别的
- 不超过几百 token

#### Layer C: 按需加载当前 app 的详细 skill

当模型判断当前请求命中 `appfs-aiim` 时，再加载详细 skill 全文。

建议第一阶段复用现有目录：

- `.claw/skills/appfs-aiim/SKILL.md`

而不是先引入新的 `.claw/apps/` 搜索根。

#### Layer D: bounded event digest

事件流不直接把原始 JSONL 全量塞上下文。

建议：

- 订阅：
  - `/_appfs/_stream/events.evt.jsonl`
  - `/<current-app>/_stream/events.evt.jsonl`
- 每轮 turn 只注入：
  - 上轮之后新增的高优先级事件摘要
  - 同类事件 collapse
  - 超量时只给 summary + count

---

### 4.2 appfs 侧：补齐“就近自描述能力”

这是这次修订里新增的重点。

如果 AppFS 侧不补“就近描述文件”，agent 侧 skill 很容易变成一层很重的补丁，因为它必须自己从大而全的 `manifest.res.json` 中推导所有控制面细节。

因此建议把 appfs 侧也纳入当前迭代：

#### 平台控制面

后续建议补：

- `/_appfs/control.res.json`

但这次可先不实现，当前 `/.well-known/appfs/runtime.json` 已经能承担第一阶段职责。

#### app 控制面

建议优先在 `/<app>/_app/` 下补这几类文件：

- `control.res.json`
  - 当前 app 的控制面概览
  - `enter_scope.act`
  - `refresh_structure.act`
  - 事件流位置

- `actions.res.json`
  - 当前 app 推荐 act 列表
  - 典型 payload
  - 推荐使用场景

- `available_scopes.res.json`
  - 可进入的 scope
  - 每个 scope 的作用
  - enter payload 示例

- `current_scope.res.json`
  - 当前 active scope
  - 主要资源路径

这几类文件的目标不是取代 `manifest.res.json`，而是做 **agent-friendly derived views**。

---

## 5. AIIM demo 的目标形态

AIIM demo 不应该只暴露：

- `contacts/zhangsan/send_message.act`

还应该同时暴露足够的“联系人解释层”和“控制面解释层”。

### 5.1 联系人层

`contacts/zhangsan/profile.res.json` 建议包含：

- `display_name`
- `contact_id`
- `aliases`
- `role`
- `status`
- `send_message_action`

这样模型在看到：

- 张三
- zhangsan
- 老张

都更容易路由到同一个 act 路径。

### 5.2 app 控制层

`_app/control.res.json` / `_app/actions.res.json` 等文件应明确告诉模型：

- 哪些 `.act` 是 app 控制面
- 哪些 `.act` 是业务动作
- 哪些 act 应该先看 profile / current_scope 再调用

### 5.3 current app skill 的数据源

AIIM skill 的生成不应只读：

- `_meta/manifest.res.json`

还应同时读取：

- `/.well-known/appfs/runtime.json`
- `/_appfs/apps.registry.json`
- `<app>/_meta/manifest.res.json`
- `<app>/_meta/app-structure-sync.state.res.json`
- `<app>/_app/control.res.json`
- `<app>/_app/actions.res.json`
- `<app>/_app/current_scope.res.json`
- `<app>/_app/available_scopes.res.json`

---

## 6. 推荐实施顺序

### P0

1. AppFS 侧补 AIIM 的控制面描述文件
2. AIIM demo 的联系人资料补齐别名/动作映射
3. appfs-agent 的 system prompt 注入 AppFS 环境摘要

### P1

4. appfs-agent 注入“当前 app skill listing”
5. 让 listing 参考 TS 版做法，暴露 `description + when_to_use`
6. 生成 `.claw/skills/appfs-aiim/SKILL.md`

### P2

7. 让模型在 AppFS 环境下按需调用当前 app skill
8. 加入当前 app 事件摘要注入

### P3

9. 再评估是否需要：
   - 多 app 默认 listing
   - 更激进的 auto-preload
   - AppFS 权限模型和 appfs-agent PermissionPolicy 联动

---

## 7. 对 v0.1 的具体修正

### 保留

- AppFS 环境摘要进 system prompt
- skill 化方向
- event stream 订阅方向

### 修改

- 不再把 `.claw/apps/<app>/SKILL.md` 作为第一步
  - 先复用 `.claw/skills/`
- 不再默认订阅所有 app 的所有事件
  - 先只订阅平台控制面 + 当前 app
- 不再把 `tail -f` 作为 runtime 设计表述
  - 改成 runtime 内部 follower / cursor reader
- 不再默认把完整 skill 文本放进 system prompt
  - 改成 listing + 按需加载

### 新增

- 明确引入“就近自描述文件”作为 appfs 侧前置能力
- 明确 AIIM demo 需要补联系人别名和控制面资源

---

## 8. 一句话版本

这轮最稳的方向不是：

> 把 AIIM 全量说明塞进 prompt

而是：

> 让 appfs 先把 AIIM 的控制面和联系人语义描述清楚，再让 appfs-agent 参考 Claude Code TS 版，用 skill listing + 按需加载的方式发现和调用它。

