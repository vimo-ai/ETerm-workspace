# Claude Code JSONL 事件模型调研

> 调研日期: 2026-02-06
> 数据来源: 全量分析 — ETerm 项目（39 文件，6392 行，20.3MB）+ 全项目（7293 文件，448773 行，1415.6MB）
> 目标: 为 iOS 端实时展示 Claude 工作过程提供数据模型依据
> 分析工具: [`docs/design/scripts/jsonl-analyzer.py`](scripts/jsonl-analyzer.py)
>
> ```bash
> # 全量分析 ETerm 项目所有 session
> python3 docs/design/scripts/jsonl-analyzer.py --format json --output report.json
>
> # 分析所有项目
> python3 docs/design/scripts/jsonl-analyzer.py --all-projects --format json --output report.json
> ```

## 1. JSONL 行类型全景

Claude Code 会话 JSONL 中包含以下行类型：

| type | 说明 | 全项目频率 | 推送价值 |
|---|---|---|---|
| `assistant` (text) | Claude 文本输出 | 25.8% of assistant | **核心** — Claude 说了什么 |
| `assistant` (thinking) | Claude 推理过程 | 33.1% of assistant | **需要** — Claude 在想什么 |
| `assistant` (tool_use) | Claude 调用工具 | 41.1% of assistant | **需要** — Claude 在做什么 |
| `user` (string content) | 用户实际提问 | - | 已有推送 |
| `user` (tool_result) | 工具返回结果 | - | **需要** — Claude 看到了什么 |
| `progress` (bash/hook/agent/mcp) | 执行进度 | 113163 行 | **可选** — loading 指示器 |
| `system` | 系统提醒注入 | 19778 行 | 跳过 |
| `summary` | 上下文压缩摘要 | 9276 行 | 跳过 |
| `file-history-snapshot` | 文件快照 | 37695 行 | 跳过 |
| `queue-operation` | 任务队列操作（subagent） | 1117 行 | 跳过 |
| `custom-title` | 会话自定义标题 | 33 行 | 跳过 |

### 全量统计

```
ETerm 项目 (39 文件, 20.3MB):
  assistant: 3319, user: 1617, file-history-snapshot: 690
  system: 459, summary: 304, queue-operation: 3

全项目 (7293 文件, 1415.6MB):
  assistant: 172649, progress: 113163, user: 95062
  file-history-snapshot: 37695, system: 19778, summary: 9276
  queue-operation: 1117, custom-title: 33

Progress 子类分布:
  bash_progress: 51076, hook_progress: 49547
  agent_progress: 9475, mcp_progress: 2571
```

## 2. Assistant 消息 Block Type 模式

### 关键发现：单一 block type

**172,649 条 assistant 消息中，仅 71 条（0.04%）包含混合 block type。** 每条 assistant 行几乎只有一种 content block：

| 模式 | ETerm 项目 | 全项目 | 说明 |
|---|---|---|---|
| `('thinking',)` | 40.4% (1342) | 33.1% (57087) | 纯推理块 |
| `('tool_use',)` | 36.1% (1197) | 41.1% (70924) | 纯工具调用 |
| `('text',)` | 23.5% (779) | 25.8% (44567) | 纯文本输出 |

混合模式分布（全项目 71 条）：
```
thinking + tool_use:           25   最常见，短 thinking 后直接调工具
text + thinking + tool_use:    20   先解释再想再调
text + thinking:               19   解释后补充思考
text + tool_use:                7   文字后直接调工具
```

### 空白 text 占位符

`text("\n\n")` 是流式输出占位符：

- **全项目 640 条**，占 text 消息的 1.4%（640/44614）
- **100% 出现在 user 消息之后**（after_user: 640）
- **ETerm 项目 0 条**（因几乎全是 Opus 4.5 模型）
- 推测仅 Opus 4.6 等较新模型产生
- **纯废数据，应跳过**

### text 内容分布

```
全项目:
  有实际内容 (≥20ch): 37,030 (83.0%)
  较短 (<20ch):         6,681 (15.0%)
  空白 (仅 \n 等):        640 ( 1.4%)
  极短 (<5ch):            263 ( 0.6%)

ETerm 项目（无空白占位）:
  有实际内容 (≥20ch):    655 (84.0%)
  较短 (<20ch):          125 (16.0%)
```

## 3. parentUuid 链式结构

所有消息通过 `parentUuid` 形成严格单链（无分叉），表示因果依赖：

```
user(question) [uuid=A]
  └→ assistant(text "\n\n") [uuid=B, parent=A]        ← 占位（仅 4.6）
       └→ assistant(thinking) [uuid=C, parent=B]
            └→ assistant(tool_use) [uuid=D, parent=C]
                 └→ progress(mcp started) [parent=D]
                      └→ progress(mcp completed) [parent=started]
                 └→ user(tool_result) [uuid=E, parent=D]
                      └→ assistant(thinking) [uuid=F, parent=E]
                           └→ assistant(text) [uuid=G, parent=F]
                                └→ assistant(tool_use) [uuid=H, parent=G]
                                     └→ ...
```

### 并行工具调用

ETerm 项目内检测到 0 组并行调用，但**全项目有 60 组**。并行工具调用表现为：
- 同一条 assistant 消息包含多个 tool_use block（混合模式中的 `text + tool_use*N` 等）
- 对应的 tool_result 也会连续出现（tool_result → tool_result 转移：10111 次）
- 同时 tool_use → tool_use 连续出现 10093 次

并行调用典型示例：
```
assistant: [thinking(41ch), tool_use(TodoWrite), tool_use(Read), tool_use(Read), tool_use(Read)]
  → tool_result (TodoWrite)
  → tool_result (Read)
  → tool_result (Read)
  → tool_result (Read)
```

### Subagent（sidechain）

全项目有 **37,673 条** 带 `agentId` 的消息（Task 工具产生的子代理链）。子代理的消息同样通过 parentUuid 链接，但通过 `agentId` 标记归属。

## 4. 事件转移矩阵

全项目 448,773 行全量统计：

```
前事件           → 后事件            次数      说明
──────────────────────────────────────────────────────────
tool_use        → tool_result      : 60,803   工具调用 → 结果
tool_result     → thinking         : 43,391   拿到结果后思考
thinking        → text             : 30,953   想完后回复
thinking        → tool_use         : 25,791   想完后调工具
text            → tool_use         : 25,591   文字后继续调工具（很常见!）
text            → user_question    : 15,156   text 是本轮最终回复
user_question   → thinking         : 11,239   提问后先思考
tool_result     → tool_result      : 10,111   连续多个结果返回（并行工具）
tool_use        → tool_use         : 10,093   连续调用（并行工具）
tool_result     → tool_use         :  8,993   结果后直接调工具（跳过 thinking）
tool_result     → text             :  6,827   结果后直接回复
user_question   → text             :  4,788   提问后直接回复（无 thinking）
text            → thinking         :  1,794   文字后再思考
tool_result     → user_question    :  1,563   结果后直接新问题
text            → text             :  1,373   连续文字
user_question   → text_empty       :    578   提问后空占位
text_empty      → thinking         :    638   空占位后思考
user_question   → tool_use         :    508   提问后直接调工具（无 thinking）
thinking        → user_question    :    320   thinking-only turn
```

ETerm 项目对比（6392 行）：

```
tool_use        → tool_result      :  1,083
tool_result     → thinking         :  1,024
thinking        → text             :    754
thinking        → tool_use         :    588
text            → tool_use         :    485
user_question   → thinking         :    219
text            → user_question    :    189
```

### 关键模式

1. **标准流程**: `user → thinking → (tool_use → tool_result → thinking)* → text`
2. **text 不是 turn 终点**: `text → tool_use` 有 25,591 次（占 text 后续的 57%），Claude 经常先写解释再调工具
3. **Turn 边界**: `text → user_question`（15,156 次）才是真正的 turn 结束
4. **~~无 stopReason~~** ⚠️ **stopReason 存在！** 详见 §6.1
5. **并行工具模式**: tool_use → tool_use 和 tool_result → tool_result 各约 10K 次，说明并行调用是常态
6. **跳过 thinking**: tool_result → tool_use（8,993 次）和 user → tool_use（508 次），说明 Claude 有时直接行动不思考

## 5. "text → tool_use" 交替模式

Claude 频繁在工具调用前写解释文字：

```
📝 "先看看改了什么代码，再用 vimo-mobile 截图验证效果。"
🔧 memex__get_session

📝 "代码改动还在。iPhone 17 模拟器在线，截个图看看当前 iOS app 状态。"
🔧 mcp_router__describe

📝 "上次截图返回 base64 太大了。让我换个方式，直接用 simctl 截图保存到本地再查看。"
🔧 Bash
```

**这意味着 text 消息有两种语义**：
- **中间文本**: 解释下一步要做什么（后接 tool_use，占 57%）
- **最终回复**: 本轮的正式回答（后接 user_question，占 34%）
- 其他：text → thinking（4%），text → text（3%）

**可通过 stopReason 区分**：最终回复的 assistant 消息携带 `stopReason: "end_turn"`，中间文本携带 `stopReason: "tool_use"` 或无 stopReason。详见 §6.1。

## 6. 特殊字段

### 6.1 stopReason ⚠️ 关键修正

**stopReason 存在于 JSONL 中**，是 turn 边界识别的关键字段：

| stopReason | 全项目次数 | ETerm 项目 | 含义 |
|---|---|---|---|
| `tool_use` | 11,535 | 357 | 本条是工具调用，等待 tool_result 后继续 |
| `end_turn` | 1,411 | 37 | **Turn 结束** — 这是最终回复 |
| `stop_sequence` | 706 | 3 | 被停止序列中断（subagent 等） |

**stopReason 出现在 summary-like 的 assistant 行上**（携带 hookCount、hasOutput 等字段，共 12,861 次全项目）。不是每条 assistant 行都有，而是 turn 的最后一条汇总行。

### 6.2 assistant 行的 top-level 字段

| 字段 | 频率 | 说明 |
|---|---|---|
| `parentUuid` | 100% | 父消息 UUID |
| `uuid` | 100% | 本消息 UUID |
| `timestamp` | 100% | ISO 8601 时间戳 |
| `requestId` | ~99% | API 请求 ID（同一轮 API 调用相同） |
| `slug` | ~98% | 模型 slug |
| `message.content` | 100% | 消息内容（content blocks 数组） |
| `stopReason` | 部分 | turn 结束标记（见 §6.1） |
| `toolUseID` | 部分 | 工具调用相关 |

### 6.3 模型分布

| 模型 | 全项目消息数 | 占比 |
|---|---|---|
| `claude-opus-4-5-20251101` | 158,839 | 92.0% |
| `claude-sonnet-4-5-20250929` | 6,543 | 3.8% |
| `claude-haiku-4-5-20251001` | 4,512 | 2.6% |
| `claude-opus-4-6` | 2,049 | 1.2% |
| `<synthetic>` | 706 | 0.4% |

ETerm 项目几乎全是 Opus 4.5（3316/3319 = 99.9%）。

### 6.4 user 行的特殊字段

| 字段 | 说明 |
|---|---|
| `thinkingMetadata` | `{maxThinkingTokens: 31999}`，标记本轮开启 thinking |
| `isCompactSummary` | 上下文被压缩的标记 |
| `permissionMode` | 权限模式 |
| `imagePasteIds` | 图片粘贴 ID（257 次全项目） |

### 6.5 progress 行的 data 结构

```json
// Bash 进度（最多，51076 次）
{ "type": "bash_progress", ... }

// Hook 进度（49547 次）
{
  "type": "hook_progress",
  "hookEvent": "SessionStart",
  "hookName": "SessionStart:clear",
  "command": "bash ..."
}

// Agent/Task 进度（9475 次）
{ "type": "agent_progress", ... }

// MCP 工具进度（2571 次）
{
  "type": "mcp_progress",
  "status": "started" | "completed",
  "serverName": "mcp-router",
  "toolName": "memex__get_session",
  "elapsedTimeMs": 16
}
```

### 6.6 summary 行

```json
{
  "type": "summary",
  "summary": "Windows 兼容性修复：ai-cli-session-db 多平台支持",
  "leafUuid": "471e5bb8-..."   // 压缩到的位置
}
```

## 7. tool_result 结构

### 正常/错误比

```
ETerm 项目: 正常 1,115 (93.2%)  错误 81 (6.8%)
全项目:     正常 64,922 (91.5%) 错误 6,017 (8.5%)
```

### 常见错误类型

```
"Exit code 137\n[Request interrupted by user for tool use]"           ← 用户中断
"The user doesn't want to proceed with this tool use..."              ← 权限拒绝
"<tool_use_error>File does not exist.</tool_use_error>"               ← 文件不存在
"<tool_use_error>Found 2 matches...but replace_all is false"          ← Edit 匹配多条
"<tool_use_error>File has not been read yet."                         ← 未先读文件
"MCP error -32603: Internal error..."                                 ← MCP 调用失败
```

### content 格式

```json
{
  "type": "tool_result",
  "tool_use_id": "toolu_016JzaNCaywbJy",
  "is_error": false,
  "content": [
    { "type": "text", "text": "..." }
  ]
}
```

## 8. tool_use 工具分布

全项目 71,000 次工具调用 Top 15：

```
Bash:           27,466 (38.7%)    Read:          15,471 (21.8%)
Edit:           11,274 (15.9%)    Grep:           5,347 ( 7.5%)
Glob:            2,731 ( 3.8%)    Write:          2,100 ( 3.0%)
TodoWrite:       2,036 ( 2.9%)    mcp_router__call: 772 ( 1.1%)
WebSearch:         465 ( 0.7%)    TaskUpdate:       434
WebFetch:          380             Task:             319
memex__get_session: 299            memex__search:    262
chrome__computer:   250
```

ETerm 项目 1,198 次：
```
Bash: 482   Read: 256   Edit: 222   TodoWrite: 74
Grep: 67    Write: 33   Glob: 30    MCP: ~26
```

## 9. Turn 结构分析

### Turn 定义

一个 Turn = 从用户提问（user string content）到下一个用户提问之间的所有事件。

### Turn 长度统计

```
全项目 (18,165 turns):
  最短: 1 事件
  最长: 1,261 事件
  平均: 14.6 事件

ETerm 项目 (228 turns):
  最短: 1 事件
  最长: 286 事件
  平均: 20.8 事件
```

### Turn 内事件分布（全项目）

```
tool_use:      71,000   最多（工具调用密集）
tool_result:   70,939   几乎与 tool_use 1:1
thinking:      57,151   大部分 turn 都有思考
text:          43,974   不含空占位
user_question: 18,165   turn 起点
mcp_started:    1,289   MCP 相关
mcp_completed:  1,203
empty_text:       640   空白占位（仅 Opus 4.6）
mcp_failed:        79   MCP 失败
```

### 典型 Turn 时间线

```
👤 user: "看下当前cwd最近两个session对话末尾内容"
  ⚪ empty_text ("\n\n")              ← 可跳过（仅 4.6）
  🤔 thinking (147ch)                ← Claude 在分析问题
  🔧 tool_use (get_recent_sessions)  ← 开始调工具
     ↳ MCP started                   ← 工具执行中
     ↳ MCP completed (16ms)          ← 工具完成
  ← tool_result                      ← 拿到结果
  🔧 tool_use (get_session)          ← 继续调工具
  ← tool_result
  🤔 thinking (327ch)                ← 分析所有结果
  📝 text: "第一个 session..."        ← 中间文字（stopReason: tool_use）
  🔧 tool_use (get_session)          ← 继续深入
  ← tool_result
  📝 text: "两个最近 session..."      ← 最终回复（stopReason: end_turn）
```

## 10. 跨模型差异

| 特征 | Opus 4.5 (92.0%) | Opus 4.6 (1.2%) | Sonnet 4.5 (3.8%) | Haiku 4.5 (2.6%) |
|---|---|---|---|---|
| `text("\n\n")` 占位 | 不产生 | 产生 | 待验证 | 待验证 |
| 混合 block type | 有（极少） | 有（极少） | 有 | 有 |
| subagent (Task) | 少用 | 常用 | 作为子代理被调用 | 作为子代理被调用 |

注：Sonnet 4.5 和 Haiku 4.5 主要以 subagent 身份出现（被 Task 工具调用），不是用户直接交互的模型。

## 11. 数据模型建议方向

基于全量调研，iOS 端实时展示需要解决的核心问题：

### 问题 1: 粒度

当前每条 JSONL 行作为独立消息推送，导致 thinking-only 和空 text 渲染为空 bubble。

**数据支撑**: 172,649 条 assistant 消息中 99.96% 是单一 block type，每条行确实只有一种语义。

### 问题 2: 语义

`text` 有"中间解释"（57%，后接 tool_use）和"最终回复"（34%，后接 user）两种语义。

**解决方案**: ~~JSONL 中无法区分~~ → **可通过 stopReason 区分**。`end_turn` = 最终回复，`tool_use` = 还会继续。

### 问题 3: Turn 边界 ✅ 已有方案

~~无 stopReason~~ → **stopReason 存在**（全项目 12,861 条）：
- `end_turn` (1,411 次) = turn 结束
- `tool_use` (11,535 次) = 中间状态
- `stop_sequence` (706 次) = 被中断

### 问题 4: 工具执行上下文

`tool_use` 和 `tool_result` 通过 `tool_use_id` 关联。并行工具调用时会有多个 tool_use 连续出现，对应的 tool_result 也会连续出现。

**数据支撑**: tool_use → tool_use 连续 10,093 次，tool_result → tool_result 连续 10,111 次。

### 问题 5: Subagent

37,673 条消息带 `agentId`，表示由 Task 工具产生的子代理。子代理使用 Sonnet/Haiku 等较小模型，消息独立但通过 agentId 归属。iOS 端需决定是否展示子代理细节。
