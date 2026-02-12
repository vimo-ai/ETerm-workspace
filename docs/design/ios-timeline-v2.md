# iOS Timeline V2 — 交互设计文档

> 创建时间: 2026-02-10
> 状态: 设计中
> 前置文档: [jsonl-event-model.md](jsonl-event-model.md)（数据模型）、[ios-timeline-status.md](ios-timeline-status.md)（V1 现状）
> 验证器: `ai-cli-session-collector/src/validate/claude/`

## 1. 设计目标

将 iOS Session 详情页从当前的 Timeline V1 升级为 V2，核心目标：

- **完整展示 Claude 每一步工作过程**：用户能看到 Claude 在想什么、说什么、做什么
- **AI 文字输出永远完整可见**：Claude 的 text 事件是叙事主线，绝不压缩
- **渐进式披露执行细节**：工具调用和思考过程可折叠，按需查看
- **统一事件渲染**：不区分"中间解释"和"最终回复"，所有 text 事件平等对待

## 2. 设计原则

1. **AI text 永远完整显示** — 无论折叠/展开状态，所有 text 事件全文可见
2. **不合并事件** — 每个事件独立渲染，Claude 输出什么就展示什么
3. **不做 finalResponse 特殊处理** — 去掉紫色色带，去掉"最终回复"判定逻辑
4. **组件样式自区分** — text 无背景、tool 卡片背景、thinking 折叠条，不靠缩进区分
5. **全部贴左对齐** — 不做缩进层级，最大化 iPhone 横向内容空间
6. **不用 emoji** — 全部使用 SF Symbols
7. **按内容类型智能缩略** — 短内容不强制压缩，长内容按语义截断，截断点因类型而异

## 3. 三层渐进披露

### 3.1 Layer 1: Turn 折叠态

用户滚动历史时的默认视图。**只隐藏执行细节（工具/thinking），保留所有 AI text。**

```
┌─────────────────────────────────────────────┐
│ User: "帮我修复这个 bug"                      │
│                                             │
│ 先看看相关文件的实现                            │
│       [5 tools · 1 thinking]                │
│ 发现问题在第 42 行，初始化顺序不对，            │
│ 应该先调用 setup() 再注册 observer             │
│       [2 tools]                             │
│ 已修复。问题出在初始化顺序，setup() 必须        │
│ 在 registerObserver() 之前调用...             │
│ ─────────────────────────────────────────── │
│ 6 tools · 2 thinking · ~3.2k tokens         │
└─────────────────────────────────────────────┘
```

组成部分：
- **用户消息**：Turn 起点
- **AI text 事件**：全部完整显示，按时间顺序排列
- **内联折叠指示器**：text 之间被折叠的工具/thinking 的统计（"5 tools · 1 thinking"）
- **统计栏**：底部总计信息（工具调用次数、thinking 次数、token 消耗等）

### 3.2 Layer 2: Turn 展开态

所有事件按时间顺序完整展示。

```
┌─────────────────────────────────────────────┐
│ User: "帮我修复这个 bug"                      │
│                                             │
│ [thinking 1247 字]                          │
│ 先看看相关文件的实现                            │
│ ┌─────────────────────────────────────────┐ │
│ │ doc.text  Read  config.swift         ✓  │ │
│ └─────────────────────────────────────────┘ │
│ ┌─────────────────────────────────────────┐ │
│ │ doc.text  Read  AppDelegate.swift    ✓  │ │
│ └─────────────────────────────────────────┘ │
│ [thinking 832 字]                           │
│ 发现问题在第 42 行，初始化顺序不对...            │
│ ┌─────────────────────────────────────────┐ │
│ │ pencil  Edit  config.swift (+3 -1)   ✓  │ │
│ └─────────────────────────────────────────┘ │
│ ┌─────────────────────────────────────────┐ │
│ │ terminal  Bash  swift build          ✓  │ │
│ └─────────────────────────────────────────┘ │
│ 已修复。问题出在初始化顺序...                    │
│ ─────────────────────────────────────────── │
│ 6 tools · 2 thinking · ~3.2k tokens         │
└─────────────────────────────────────────────┘
```

事件类型渲染规则（详见 §4）：
- **text**：完整 Markdown 渲染，无背景
- **tool_use**：紧凑卡片行（icon + 工具名 + 智能摘要 + 状态），点击进入 Layer 3
- **thinking**：折叠条（brain icon + 字数），可独立展开查看全文
- **系统事件**：幽灵化样式（详见 §5）

### 3.3 Layer 3: 单步详情

点击展开态中的工具行，进入对应的专用 ToolView：

- Bash → 终端风格（黑底绿字命令 + 输出）
- Edit → Diff 视图（红删绿增）
- Read → 语法高亮代码预览
- Grep/Glob → 搜索结果列表
- 其他工具 → 各自的专用视图

已有 13 个 ToolView 组件可直接复用。

## 4. 事件类型渲染规格

### 4.1 text（AI 文字输出）

- **永远完整显示**，折叠/展开态都可见
- Markdown 渲染（使用 MarkdownUI + Highlightr）
- 无背景色，全宽度
- 支持文本选择

### 4.2 tool_use（工具调用）

紧凑行样式：

```
┌─────────────────────────────────────────┐
│ [icon]  ToolName  summary          [状态] │
└─────────────────────────────────────────┘
```

- 浅灰色圆角背景，与 text 形成视觉区分
- 左侧：工具类型对应的 SF Symbol icon
- 工具名：monospaced 字体
- 摘要：按工具类型智能生成（见下表）
- 右侧：状态指示（spinner/checkmark/error/timeout）
- 点击：展开为对应的 ToolView 详情

摘要生成规则（保持现有 CompactToolRow 逻辑）：

| 工具 | 摘要内容 |
|------|---------|
| Read/Edit/Write | 文件名（lastPathComponent） |
| Bash | 命令首行，截断到合理长度 |
| Grep/Glob | pattern 参数 |
| WebSearch | query 参数 |
| Task | description 参数 |
| 其他 | displayText 或 formattedInput 首行 |

### 4.3 tool_result（工具返回）

不独立显示。与对应的 tool_use 合并（通过 tool_use_id 匹配）：
- 成功 → tool_use 行右侧显示 checkmark
- 失败 → tool_use 行右侧显示 error icon，展开后可见错误信息

### 4.4 thinking（思考过程）

折叠条样式：

```
[brain icon]  思考  (1247 字)               [展开/收起]
```

- 默认折叠，点击展开查看全文
- 展开后的文本使用次级颜色、略小字号
- 紫色调视觉标识

### 4.5 系统事件（详见 §5）

### 4.6 user_text（用户消息）

- Turn 起点
- 视觉上与 AI 内容明确区分（靠右、蓝色调、或其他方式）
- 支持图片显示（用户粘贴的截图）

## 5. 系统事件处理

系统事件是环境反馈，不是 Claude 的主动行为。视觉上应可见但不抢夺注意力。

### 5.1 样式

- **字号**：caption / footnote 级别
- **颜色**：secondary 文字色，可加 0.7 opacity
- **图标**：线描风格 SF Symbols，无填充
- **背景**：无
- **错误状态**：仅 error 级别使用淡红色文字

### 5.2 类型

| 系统事件 | 说明 | 显示策略 |
|---------|------|---------|
| hook_result | Hook 执行结果 | 正常显示 |
| progress (bash) | Bash 执行进度 | 量大（51K），考虑只在活跃态显示 |
| progress (mcp) | MCP 工具进度 | 简要显示（started/completed + 耗时） |
| progress (agent) | 子代理进度 | 关联到 SubagentRow |
| system error/warn | 系统警告 | error 级别提亮 |
| compact_boundary | 上下文压缩 | 已有 CompactionDivider |

### 5.3 折叠态

系统事件在折叠态隐藏（与工具/thinking 一起折叠）。

## 6. 状态与交互

### 6.1 Turn 折叠/展开行为

| 场景 | 默认状态 | 说明 |
|------|---------|------|
| 实时推送的 Turn | 展开 | 用户正在看，不自动收起 |
| 历史加载的 Turn | 折叠 | 用户翻阅历史，需要快速浏览 |
| Turn 完成时 | 保持当前状态 | 不自动折叠，用户手动操作 |
| 用户手动操作 | 切换 | 随时可折叠/展开任何 Turn |

### 6.2 活跃态视觉

Turn 进行中时：
- 事件逐条追加，自动滚动跟进
- 最新事件附近有 spinner 或轻微脉冲指示
- thinking 条可显示小型 ProgressView

Turn 完成时：
- spinner 停止
- 内容保持不变，不自动折叠
- 用户可选择手动收起

### 6.3 完成态视觉

已完成的 Turn 与活跃 Turn 视觉上无需强制区分。活跃态通过 spinner 即可辨识。

## 7. 统计栏

位于每个 Turn 底部，折叠/展开态都显示。

内容：
- 工具调用次数（可按类型细分：3 edits · 2 reads · 1 bash）
- thinking 次数
- token 消耗（待确认数据可用性）
- 可选：Turn 耗时

样式：
- 小字号、次级颜色
- 顶部细分隔线
- 不可交互（纯信息展示）

## 8. 需确认的数据问题

1. **token 消耗**：JSONL 中 assistant 消息的 usage 字段是否可用？需查验证器 schema
2. **Turn 耗时**：首条事件到末条事件的时间差可直接计算
3. **progress 事件**：是否从 VlaudeKit 推送到 iOS？当前推送链路是否包含 progress 类型？

## 9. 已有组件复用清单

| 组件 | 状态 | V2 用途 |
|------|------|---------|
| CompactToolRow | 可复用 | Layer 2 工具行 |
| ToolExecutionBubble | 可复用 | Layer 3 详情路由 |
| 13 个 ToolView | 可复用 | Layer 3 各工具详情 |
| ThinkingEventView | 可复用 | Layer 2 thinking 折叠条 |
| SubagentRow | 可复用 | 子代理展示 |
| CompactionDivider | 可复用 | 上下文压缩分隔 |
| ExecutionSummaryBar | **废弃** | 被新的折叠/展开机制取代 |
| TurnCard 紫色色带 | **废弃** | 不再区分 finalResponse |
| TurnCard activitySection | **重写** | 新的事件列表渲染 |
| Turn.finalResponse | **废弃** | 不再需要 |
| Turn.activityEvents | **重写** | 不再过滤 final text |

## 10. 与 V1 的关键差异

| 维度 | V1 | V2 |
|------|----|----|
| 折叠态 | Turn 级别折叠为 ExecutionSummaryBar | text 保留，只折叠执行细节 |
| 最终回复 | 紫色色带独立渲染 | 与其他 text 平等，无特殊处理 |
| text 事件 | 活跃时灰色小字，完成后隐藏 | 永远完整 Markdown 渲染 |
| 执行过程 | 活跃时平铺，完成后一键折叠 | 用户控制折叠/展开 |
| finalResponse 判定 | 最后一条 isFinal text | 不需要 |
