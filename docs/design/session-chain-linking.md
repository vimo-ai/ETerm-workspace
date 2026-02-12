# 多源会话数据结构与采集规范

> 创建时间: 2026-02-09
> 状态: 调研完成
> 关联文档: [agent-v2-rw-separation.md](./agent-v2-rw-separation.md)

## 1. 概述

ai-cli-session-collector 支持四个 AI CLI 工具的会话数据采集，统一存入 ai-cli-session-db。本文档梳理每个数据源的磁盘结构、JSONL/JSON 格式、session 生命周期特性，以及代码层面的处理现状和改进方向。

### 1.1 数据规模（截至 2026-02-09）

| 来源 | DB sessions | DB 消息 | 磁盘文件 | 适配器 | 入库路径 |
|------|------------|---------|---------|--------|---------|
| Claude Code | 9,333 | 401,120 | 2,266 JSONL + 508 subagent 目录 | `ClaudeAdapter` | collector 扫描 + VlaudeKit 推送 |
| Codex CLI | 10,049 | 150,837 | 594 JSONL + history.jsonl | `CodexAdapter` | collector 扫描 |
| Gemini CLI | 53 | 1,897 | 57 JSON | `GeminiAdapter` | collector 扫描 |
| OpenCode | 1 | 4 | session/message/part JSON | `OpenCodeAdapter` | collector 扫描 |

### 1.2 DB 来源区分

```
projects.source  → 'claude' | 'codex' | 'opencode' | 'gemini'
sessions.channel → 'code' (Claude) | 'cli' (Codex/Gemini) | NULL (OpenCode)
```

`source` 在 projects 表，不在 sessions 表。查询时需 JOIN。

---

## 2. Claude Code

### 2.1 磁盘结构

```
~/.claude/projects/
├── {encoded-project-path}/               # 项目目录（路径编码，如 -Users-foo-myproject）
│   ├── sessions-index.json               # 项目级 session 索引（Claude 自身维护）
│   ├── {uuid}.jsonl                      # session 主文件
│   └── {uuid}/                           # session 附属目录（可选）
│       ├── subagents/                    # subagent 对话（可选）
│       │   ├── agent-a{hash}.jsonl       # 普通 subagent（Task 工具产生）
│       │   ├── agent-aprompt_suggestion-{hash}.jsonl  # 输入建议 agent
│       │   └── agent-acompact-{hash}.jsonl            # 压缩摘要生成 agent
│       └── tool-results/                 # 大型工具输出（可选）
│           └── toolu_{id}.txt            # 超出内联阈值的工具结果
└── ...

ETerm 项目统计：91 个项目目录，2,266 个主 JSONL，234 个附属目录
  - 166 个含 subagents/（1,233 普通 + 93 prompt_suggestion + 31 compact）
  - 110 个含 tool-results/（1,263 个 .txt 文件，中位数 2.8KB，最大 1MB）
```

#### 附属目录详解

**subagents/**：三种命名模式

| 模式 | 数量 | 说明 |
|------|------|------|
| `agent-a{7位hash}.jsonl` | 1,233 | 普通 Task subagent（Explore/Plan/engineer 等） |
| `agent-aprompt_suggestion-{hash}.jsonl` | 93 | 输入建议（预测用户下一步输入，`isSidechain: true`） |
| `agent-acompact-{hash}.jsonl` | 31 | 压缩摘要生成（3 行：system prompt + 压缩指令 + AI 生成摘要） |

所有 subagent 的 `isSidechain: true`，`sessionId` 指向父 session UUID。

**关联方式**：纯目录结构关联（`{session}/subagents/agent-{agentId}.jsonl`）。`sourceToolUseID` 和 `parentToolUseID` 字段在实际数据中**均为空**，不可用于关联。

**tool-results/**：大型工具输出的外部存储

当工具输出超过内联阈值时，JSONL 中的 `tool_result` block content 变为：
```
<persisted-output>
Output too large (26.3KB). Full output saved to: {session-dir}/tool-results/toolu_{id}.txt
</persisted-output>
```
同时 `toolUseResult` 字段中包含 `{ "type": "text", "file": { "filePath": "...", "content": "..." } }`。

### 2.2 sessions-index.json

Claude Code 自身维护的索引文件，每个项目一份：

```json
{
  "version": 1,
  "entries": [{
    "sessionId": "uuid",
    "fullPath": "/absolute/path/to/{uuid}.jsonl",
    "fileMtime": 1770359788297,
    "firstPrompt": "用户首条输入",
    "customTitle": null,
    "summary": "AI 生成的摘要",
    "messageCount": 631,
    "created": 1770350463097,
    "modified": 1770359788297,
    "gitBranch": "main",
    "projectPath": "/Users/.../project",
    "isSidechain": false
  }],
  "originalPath": "/Users/.../project"
}
```

**关键发现**：索引中**没有** `parentSessionId`、`continuedFrom` 等关联字段。Claude Code 自身不维护跨 session 链路。

### 2.3 JSONL 格式

每行一个 JSON 对象。核心字段：

**消息字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | `user` / `assistant` / `system` / `summary` / `custom-title` / `progress` / `file-history-snapshot` / `queue-operation` |
| `message` | object | Anthropic API 格式 `{ id, role, content, stop_reason, stop_sequence, usage, model, type, context_management, container }` |
| `uuid` | string | 消息唯一 ID |
| `parentUuid` | string | 父消息 UUID（对话树结构） |
| `timestamp` | string | ISO8601 或毫秒时间戳 |
| `sessionId` | string | 所属 session ID |

**上下文字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `cwd` | string | 工作目录（user 消息必有） |
| `gitBranch` | string | Git 分支 |
| `isSidechain` | bool | 是否为侧链 |
| `permissionMode` | string | 权限模式 |
| `slug` | string | 模型标识 |
| `version` | string | CLI 版本号 |

**AI 响应字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `requestId` | string | API 请求 ID |
| `stopReason` | string | `end_turn` / `tool_use` / `max_tokens` / `""` (空字符串=未结束) |
| `durationMs` | number | 响应耗时 |
| `thinkingMetadata` | object | 思考元数据 |
| `isApiErrorMessage` | bool | assistant 行标记：此消息是 API 错误（`true` 时伴随 `error` 字段） |
| `error` | string\|object | assistant 行：错误标识（如 `"authentication_failed"`）；system/api_error 行：完整错误对象 |

**压缩相关字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `isCompactSummary` | bool | 压缩摘要标记 |
| `isVisibleInTranscriptOnly` | bool | 仅在 transcript 中可见 |
| `logicalParentUuid` | string | 压缩边界逻辑父 UUID |
| `subtype` | string | 子类型（如 `compact_boundary`、`turn_duration`） |
| `compactMetadata` | object | `{ trigger, preTokens }` |
| `microcompactMetadata` | object | 微压缩元数据 `{ trigger, preTokens, tokensSaved, compactedToolIds, clearedAttachmentUUIDs }` |

**工具/Subagent 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `toolUseID` | string | 工具调用 ID |
| `toolUseResult` | object | 工具执行结果 |
| `sourceToolUseID` | string | subagent 来源工具 ID |
| `sourceToolAssistantUUID` | string | subagent 来源 assistant UUID |
| `parentToolUseID` | string | 父工具调用 ID |

**用户输入附加字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `imagePasteIds` | array | user 行：用户粘贴的图片 ID 列表（如 `[1]`, `[2]`），对应 message.content 中的 image block |
| `planContent` | string | user 行：用户在 plan mode 中编辑的计划内容（Markdown 格式） |

**file-history-snapshot 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `messageId` | string | 关联的消息 UUID（快照对应哪条消息修改的文件） |

**summary 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `leafUuid` | string | 对话树的叶节点 UUID（AI 生成摘要时的最新消息） |

**queue-operation 字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `operation` | string | 操作类型：`enqueue` / `dequeue` / `remove` / `popAll` |

**api_error 重试字段**（system/api_error 子类型专有）：

| 字段 | 类型 | 说明 |
|------|------|------|
| `cause` | object | 错误原因 `{ code, path, errno }` |
| `retryAttempt` | number | 当前重试次数 |
| `maxRetries` | number | 最大重试次数（通常 10） |
| `retryInMs` | number | 下次重试等待毫秒数 |

**其他字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `isMeta` | bool | 元消息标记 |
| `userType` | string | 用户输入类型 |
| `todos` | array | TODO 列表 |
| `hookCount` / `hookErrors` / `hookInfos` | | hook 相关 |
| `preventedContinuation` | bool | 阻止继续标记 |
| `hasOutput` | bool | 是否有输出 |
| `data` | object | 附加数据 |
| `snapshot` / `isSnapshotUpdate` | | 快照相关 |
| `level` | string | system 消息级别（如 `"suggestion"`） |

### 2.4 JSONL 行模型：一 Block 一行

**关键发现**：Claude Code 的 JSONL **不是**"一条消息一行"，而是**"一个 content block 一行"**。

一次 API 响应（assistant）可能产生多行 JSONL：

```
行1: { type: "assistant", message: { content: [{ type: "thinking", ... }] }, requestId: "req_abc", parentUuid: "u1", uuid: "u2" }
行2: { type: "assistant", message: { content: [{ type: "text", ... }] },    requestId: "req_abc", parentUuid: "u2", uuid: "u3" }
行3: { type: "assistant", message: { content: [{ type: "tool_use", ... }] }, requestId: "req_abc", parentUuid: "u3", uuid: "u4" }
```

**requestId** 将同一次 API 响应的多行 JSONL 关联为一组。同一 requestId 内的行通过 `parentUuid` 串成链表。

**stop_reason** 仅在 API 响应的**最后一行**的 `message.stop_reason` 中出现（非空），其余行的 `message.stop_reason` 为 `null`。

**message.usage** 也在 assistant 消息中（嵌入 message 对象），包含：
- `input_tokens`、`output_tokens`
- `cache_creation_input_tokens`、`cache_read_input_tokens`
- `server_tool_use` 子对象（web_search 等）

**message 子字段完整清单**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | string | Anthropic API 消息 ID |
| `role` | string | `user` / `assistant` |
| `content` | string\|array | 纯字符串或 ContentBlock 数组 |
| `stop_reason` | string\|null | `end_turn` / `tool_use` / `max_tokens` / `null` |
| `stop_sequence` | any\|null | 停止序列（通常 null） |
| `usage` | object | token 用量统计 |
| `model` | string | 模型 ID |
| `type` | string | 消息类型标识 |
| `context_management` | null | 上下文管理（目前恒为 null，预留字段） |
| `container` | null | 容器（目前恒为 null，预留字段） |

**Content Block 类型清单**：

| block type | 字段 | 出现位置 | 说明 |
|------------|------|---------|------|
| `thinking` | `type, thinking, signature` | assistant | 思考过程（signature 用于验证） |
| `text` | `type, text, citations?` | assistant | 文本输出（citations 为可选引用） |
| `tool_use` | `type, id, name, input` | assistant | 工具调用 |
| `tool_result` | `type, tool_use_id, content?, is_error?` | user (tool_result) | 工具执行结果 |
| `image` | `type, source: { type, media_type, data }` | user | 用户粘贴的图片（base64 编码，伴随 `imagePasteIds`） |

### 2.5 parentUuid 对话树

`parentUuid` 构成一棵**对话树**（非线性链表），有三种分叉/断裂场景：

#### 场景 1：并行工具调用分叉

```
u1 (assistant/text "让我并行搜索")
├── u2 (assistant/tool_use "grep ...")     parentUuid=u1
│   └── u3 (user/tool_result)             parentUuid=u2
└── u4 (assistant/tool_use "glob ...")     parentUuid=u1  ← 分叉！与 u2 共享父节点
    └── u5 (user/tool_result)             parentUuid=u4
```

当 assistant 在同一请求中发起多个 `tool_use`，每个 tool_use 的 `parentUuid` 都指向前一个文本 block。对应的 `tool_result` 的 `parentUuid` 指向各自的 `tool_use`。

#### 场景 2：Compaction 断裂

```
... → u100 (正常对话) → u101 (正常对话)
                                |
                                × (parentUuid 链断裂)
                                |
u200 (system/compact_boundary)    parentUuid=根节点(first user message)  ← 跳回根！
  └── u201 (user/isCompactSummary=true)    parentUuid=u200
       └── u202 (assistant 继续对话)       parentUuid=u201
```

compact_boundary 的 `parentUuid` 指向 session 的**第一条用户消息**（根节点），不是上一条消息。通过 `logicalParentUuid` 维持逻辑连续性。

#### 场景 3：正常线性

大多数情况 parentUuid 形成简单链表：`user → thinking → text → tool_use → tool_result → thinking → text → ...`

### 2.6 Turn 结构

一个 Turn 是用户一次输入到 AI 最终响应完成的完整过程：

```
Turn 开始 ─────────────────────────────────────────────────
  user (type: "user", message.content = string)           # 用户输入

  assistant/thinking  ─┐
  assistant/text       │ requestId 分组（一次 API 调用）
  assistant/tool_use  ─┘

  user/tool_result (type: "user", toolUseResult 非空)      # 工具执行结果

  assistant/thinking  ─┐
  assistant/text       │ 下一次 API 调用（可能多轮 tool → result）
  assistant/tool_use  ─┘

  user/tool_result ...

  assistant/thinking  ─┐
  assistant/text       │ 最终响应（stop_reason = "end_turn"）
                      ─┘

  system (subtype: "stop_hook_summary")                    # hook 结果（可选）
  system (subtype: "turn_duration", data: { durationMs })  # Turn 耗时统计
Turn 结束 ─────────────────────────────────────────────────
```

Turn 边界判定：**下一个 `type: "user"` 且 `message.content` 为 string**（非 tool_result）表示新 Turn 开始。

### 2.7 system 消息子类型清单

| subtype | 数量 | 说明 |
|---------|------|------|
| `stop_hook_summary` | 5,629 | 用户 hook 执行结果摘要 |
| `turn_duration` | 2,685 | Turn 耗时，`data.durationMs` |
| `compact_boundary` | 163 | 上下文压缩边界 |
| `api_error` | 94 | API 调用错误 |
| `local_command` | 66 | 本地命令执行 |
| `microcompact_boundary` | 17 | 微压缩边界（轻量级压缩） |

注：system 消息的 `type` 为 `"system"`，`message` 字段可能缺失或为简单字符串。

### 2.8 上下文压缩机制（Compaction）

Context overflow 是**文件内操作**，不产生新 session。

当上下文窗口耗尽时，Claude Code 在同一个 JSONL 文件中追加两条特殊消息：

**1) 压缩边界**（`type: system, subtype: compact_boundary`）：

```json
{
  "type": "system",
  "subtype": "compact_boundary",
  "content": "Conversation compacted",
  "logicalParentUuid": "d0faa648-...",
  "compactMetadata": { "trigger": "auto", "preTokens": 167146 },
  "uuid": "535a5703-..."
}
```

**2) 压缩摘要**（`type: user, isCompactSummary: true`）：

```json
{
  "type": "user",
  "isCompactSummary": true,
  "isVisibleInTranscriptOnly": true,
  "message": {
    "content": [{
      "text": "This session is being continued from a previous conversation that ran out of context. Below is a summary..."
    }]
  }
}
```

**parentUuid 影响**（详见 2.5 场景 2）：
- compact_boundary 的 `parentUuid` 跳回根节点（session 第一条 user message）
- `logicalParentUuid` 指向压缩前最后一条消息，维持逻辑连续性
- 压缩后对话在**同一 JSONL 文件**中继续，session ID 不变
- `compactMetadata.preTokens` 记录压缩前 token 数

**microcompact_boundary**（17 次）：轻量级压缩，行为类似但规模更小。

### 2.9 Rollout Session（热更新临时 session）

Claude Code 热更新（binary rollout）期间会产生临时 session：

| 特征 | 说明 |
|------|------|
| session_id 格式 | `rollout-{ISO-timestamp}-{uuid7}` |
| 磁盘文件 | **不存在**（从不写入 JSONL） |
| 入库路径 | VlaudeKit 实时推送（非 collector） |
| DB channel | `code` |
| DB projects.source | `claude` |
| encoded_dir_name | 空 |
| 数量 | 85 个（截至 2026-02-09） |

**数据重复性：待验证**。初步调查显示部分 rollout session 的消息时间线与主 session 重叠，但需要更精确的内容级对比来确认是否 100% 重复。

**注意命名混淆**：Codex CLI 的磁盘文件也使用 `rollout-{timestamp}-{uuid}.jsonl` 格式，但 Codex 的 DB session_id 只取 UUID 部分。两者恰好共享命名模式，但完全不同：

| | Claude Rollout (DB) | Codex 文件 (磁盘) |
|---|---|---|
| session_id | `rollout-2026-02-09T10-08-23-019c4028-...` | `019c4028-390f-7e20-...` |
| channel | `code` | `cli` |
| source | `claude` | `codex` |
| 磁盘文件 | 无 | `~/.codex/sessions/YYYY/MM/DD/rollout-...jsonl` |

### 2.10 代码现状（ClaudeAdapter）

- 位置：`ai-cli-session-collector/src/adapter/claude.rs`
- 支持全量 + 增量采集（实现 `IncrementalAdapter`）
- 跳过 `file-history-snapshot`、`progress`、`queue-operation` 类型
- `JsonlEntry` 定义了 `is_compact_summary`、`is_visible_in_transcript_only`、`is_meta`、`tool_use_result` 但标记 `#[allow(dead_code)]` 未使用
- 通过 `#[serde(flatten)] extra` 捕获所有未定义字段

**待改进**：
- 利用 `isCompactSummary` 统计 session 压缩次数
- 利用 `isVisibleInTranscriptOnly` 在展示层过滤压缩摘要
- 利用 `compactMetadata` 记录 token 消耗

### 2.11 消费端容错处理指南

验证器在 461K 行真实数据上发现以下边界 case（0 个未知字段，3774 个警告）。所有消费端（adapter、VlaudeKit、MemexKit、iOS Timeline）需要对这些 case 做防御性处理。

#### 2.11.1 消费端必须兼容的 case

| 现象 | 数量 | 原因 | 处理方式 |
|------|------|------|---------|
| assistant 行缺少 `requestId` | 350 | 早期版本 Claude Code 不写 requestId | 当作**独立单行响应**处理，不参与 requestId 分组。分组逻辑必须容忍 `requestId` 缺失 |
| `stopReason` / `message.stop_reason` 不在 requestId 分组的最后一行 | 760 | 旧版写入顺序不同 | 分组内**全量扫描** stop_reason，不要假设只在末行 |
| compaction 产生的"假 Turn" | 2170 | `isCompactSummary=true` 的 user 行被识别为 Turn 开头 | Turn 检测时**跳过** `isCompactSummary=true` 的行，它不代表真正的用户输入 |
| `tool_use` 后缺少 `tool_result` | 3 | 会话中断（用户 Ctrl-C、崩溃） | 标记为"已中断"，Timeline 上显示未完成状态，不 crash |
| `<persisted-output>` 引用的文件不存在 | 114 | 用户清理文件、写入中断 | 显示 fallback 文本（如"工具输出已丢失"），不 crash |

#### 2.11.2 可直接忽略的 case

| 现象 | 数量 | 原因 | 说明 |
|------|------|------|------|
| 项目目录下非 UUID 格式子目录 | 191 | 其他工具或用户创建 | 扫描 session 时按 UUID 格式匹配过滤即可 |
| 磁盘 session 不在 `sessions-index.json` 中 | 33 | index 未及时更新 | 以磁盘文件为准，index 仅作加速用 |
| subagent 的 `sessionId` 与父 session 不一致 | 8 | 数据异常 | 按目录结构关联 subagent（`{session}/subagents/`），不依赖 `sessionId` 字段 |

#### 2.11.3 压缩链路容错

| 现象 | 数量 | 原因 | 处理方式 |
|------|------|------|---------|
| `compact_boundary` 后缺少 `isCompactSummary` 摘要行 | 27 | 压缩未完成（中断或异常） | 视为不完整压缩，保留 boundary 之前的原始数据继续线性阅读 |
| `logicalParentUuid` 指向不存在的 uuid | 91 | 跨 fork 压缩或数据损坏 | 放弃该分支的 fork 回溯，仅做线性阅读。不影响 Turn 级别展示 |
| `microcompact_boundary` 缺少 `logicalParentUuid` | 27 | 微压缩不一定标记 parent | 当作普通分隔符处理，不做 fork 定位 |

#### 2.11.4 通用原则

1. **永远不要 crash**：任何字段缺失或格式异常都应 graceful fallback，不能让单条数据问题导致整个 session 不可用
2. **以磁盘文件为准**：`sessions-index.json` 只是辅助索引，不作为唯一数据源
3. **以目录结构为准**：subagent 关联使用目录路径（`{session}/subagents/`），不依赖 JSONL 内部字段
4. **全量扫描代替假设**：`stopReason`、`requestId` 等关键字段的位置不可预设，必须遍历查找
5. **压缩行不参与业务逻辑**：`isCompactSummary=true` 的行只用于恢复上下文，不应作为 Turn 起点、不计入消息条数

---

## 3. Codex CLI

### 3.1 磁盘结构

```
~/.codex/
├── history.jsonl                     # 会话索引（session_id + timestamp + text）
├── sessions/
│   └── {year}/{month}/{day}/
│       └── rollout-{timestamp}-{uuid7}.jsonl  # session 事件流
├── config.toml                       # 配置
├── auth.json                         # 认证
├── models_cache.json                 # 模型缓存
├── skills/                           # 技能配置
├── rules/                            # 规则
└── log/                              # 日志

共 594 个 session 文件，history.jsonl 1,449 条记录
```

### 3.2 history.jsonl 格式

```json
{ "session_id": "019a1fa0-f318-7213-b2b3-39bc3c7defc1", "ts": 1768452489295, "text": "用户首条输入" }
```

注意：session_id 是 UUID7 格式（含时间信息），与文件名中的 UUID 部分一致。

### 3.3 Session 事件流格式

每行一个事件：

```json
{ "timestamp": "ISO8601", "type": "event_type", "payload": { ... } }
```

事件类型：

| type | 说明 | payload 关键字段 |
|------|------|-----------------|
| `session_meta` | 会话元数据 | `id, cwd, cli_version, source, git: { commit_hash, branch, repository_url }` |
| `turn_context` | 对话上下文 | `cwd, model, approval_policy, sandbox_policy, summary` |
| `response_item` | AI 响应 | `type` 子类型决定具体格式（见下表） |
| `compacted` | 上下文压缩 | `message`（摘要文本） |
| `event_msg` | 事件消息 | 与 response_item 部分重复，跳过 |

response_item 子类型：

| item type | 说明 | 关键字段 |
|-----------|------|---------|
| `message` | 用户/助手消息 | `role, content: [{ type, text }]` |
| `reasoning` | 推理过程 | `summary: [{ text }], encrypted_content` |
| `function_call` | 函数调用 | `name, call_id, arguments` |
| `function_call_output` | 函数输出 | `call_id, output` |
| `custom_tool_call` | 自定义工具 | `name, id, arguments` |
| `custom_tool_call_output` | 自定义工具输出 | `id, output` |
| `ghost_snapshot` | 快照 | payload |

### 3.4 代码现状（CodexAdapter）

- 位置：`ai-cli-session-collector/src/adapter/codex.rs`
- 仅实现 `ConversationAdapter`（不支持增量采集）
- 通过 `history.jsonl` 索引 session，按日期目录查找文件
- 全量解析所有事件类型

**待改进**：
- 594 个磁盘文件 vs 85 个已入库（history.jsonl 只有 1,449 条记录，但 DB 有 10,049 — 大部分来自增量 collector 未经 history 索引的直接扫描）
- 考虑实现 `IncrementalAdapter` 支持增量采集

---

## 4. Gemini CLI

### 4.1 磁盘结构

```
~/.gemini/tmp/
├── {project-hash}/                   # 项目目录（SHA-256 哈希）
│   ├── logs.json                     # 简单用户消息日志（未使用）
│   └── chats/
│       └── session-{timestamp}.json  # 完整会话记录
└── ...

共 57 个 session 文件
```

### 4.2 Session JSON 格式

```json
{
  "sessionId": "uuid",
  "projectHash": "sha256-hash",
  "startTime": "ISO8601",
  "lastUpdated": "ISO8601",
  "messages": [{
    "id": "msg-uuid",
    "timestamp": "ISO8601",
    "type": "user" | "gemini" | "info" | "error",
    "content": "消息内容",
    "model": "gemini-2.5-pro",
    "thoughts": [{ "subject": "...", "description": "...", "timestamp": "..." }],
    "toolCalls": [{ "id": "...", "name": "...", "args": {}, "result": {}, "status": "..." }],
    "tokens": { "input": 100, "output": 200, "cached": 50, "thoughts": 30, "total": 380 }
  }]
}
```

### 4.3 代码现状（GeminiAdapter）

- 位置：`ai-cli-session-collector/src/adapter/gemini.rs`
- 单 JSON 文件包含完整会话（非流式）
- 跳过 `info`/`error` 类型的系统消息
- project_path 使用 `gemini:{hash}` 格式（无真实路径信息）

**待改进**：
- Gemini CLI 不存储 cwd，导致无法关联真实项目路径

---

## 5. OpenCode

### 5.1 磁盘结构

```
~/.local/share/opencode/storage/
├── session/{projectID}/ses_{sessionID}.json    # 会话元数据
├── message/{sessionID}/msg_{messageID}.json    # 消息
├── part/{messageID}/prt_{partID}.json          # 消息内容块
├── project/                                     # 项目信息
├── session_diff/                                # 会话差异
├── todo/                                        # TODO
└── migration/                                   # 迁移记录
```

### 5.2 数据格式

**Session JSON**: `{ id, slug, version, projectID, directory, title, time: { created, updated }, summary: { additions, deletions, files } }`

**Message JSON**: `{ id, sessionID, role, time: { created, completed }, parentID, modelID, providerID, mode, agent, path: { cwd, root }, cost, tokens: { input, output, reasoning, cache }, finish }`

**Part JSON**: `{ id, sessionID, messageID, type: "text"|"tool"|"reasoning", text, callID, tool, state: { status, input, output }, hash, snapshot, files }`

### 5.3 代码现状（OpenCodeAdapter）

- 位置：`ai-cli-session-collector/src/adapter/opencode.rs`
- 三层文件结构（session → message → part），需多次磁盘读取
- 仅 1 个 session 入库，使用率最低

---

## 6. Rollout Session 问题

### 6.1 问题定义

DB 中有 85 个 `session_id LIKE 'rollout-%'` 的 session，全部属于 `claude` source。它们由 VlaudeKit 在 Claude Code 热更新期间实时推送入库，磁盘上无对应 JSONL 文件。

### 6.2 处理方案

**选项 A: VlaudeKit 入库时过滤**（推荐）

`readAndPushNewMessages` 检查 `sessionId` 前缀，跳过 `rollout-` session。从源头杜绝脏数据。

**选项 B: 查询层过滤**

所有消费者查询时加 `WHERE s.session_id NOT LIKE 'rollout-%'` 或 JOIN `projects.source` 过滤。

**历史数据清理**：

```sql
-- 预览
SELECT s.session_id, s.message_count, p.name
FROM sessions s JOIN projects p ON s.project_id = p.id
WHERE s.session_id LIKE 'rollout-%';

-- 清理
DELETE FROM messages WHERE session_id IN (SELECT session_id FROM sessions WHERE session_id LIKE 'rollout-%');
DELETE FROM sessions WHERE session_id LIKE 'rollout-%';
```

### 6.3 待验证

rollout session 的消息与主 session 是否完全重复。如果存在独有数据，则需要合并策略而非简单删除。

---

## 7. 代码改进方向

### 7.1 ClaudeAdapter - 利用未使用字段

当前 `JsonlEntry` 中 `is_compact_summary`、`is_visible_in_transcript_only`、`is_meta`、`tool_use_result` 均标记 `#[allow(dead_code)]`。

可利用方向：
1. `isCompactSummary` → sessions 表增加 `compact_count`（压缩次数，反映对话深度）
2. `compactMetadata.preTokens` → 统计 session 实际 token 消耗
3. `isVisibleInTranscriptOnly` → 展示层过滤压缩摘要
4. `logicalParentUuid` → timeline 视图的逻辑连续性

### 7.2 sessions 表增加 source 字段

当前 `source` 仅在 `projects` 表，查询需 JOIN。考虑在 `sessions` 表冗余 `source` 字段，简化查询和过滤。

### 7.3 统一命名规范

Codex 磁盘文件名 `rollout-{ts}-{uuid}` 与 Claude Code rollout session_id 格式相同，造成分析混淆。代码注释中应明确标注两者区别。

---

## 8. 附录：适配器架构

```
                    ConversationAdapter (trait)
                    ├── meta()          → &AdapterMeta
                    ├── data_path()     → &Path
                    ├── list_sessions() → Vec<SessionMeta>
                    └── parse_session() → Option<ParseResult>
                         │
              ┌──────────┼──────────────┬──────────────┐
              │          │              │              │
       ClaudeAdapter  CodexAdapter  GeminiAdapter  OpenCodeAdapter
       (+ IncrementalAdapter)
              │          │              │              │
     ~/.claude/projects  ~/.codex   ~/.gemini/tmp  ~/.local/share/opencode
       JSONL (流式)    JSONL (事件流)  JSON (完整)    JSON (三层)
```

注册入口：`adapter/mod.rs` → `all_adapters()` → `[Claude, Codex, OpenCode, Gemini]`

数据流：适配器 → `SessionMeta` + `ParseResult` → `ai-cli-session-db` → SQLite
