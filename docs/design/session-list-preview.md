# iOS 消息数据设计

## 1. 场景矩阵

| # | 页面 | 场景 | 数据需求 | 当前状态 | 备注 |
|---|------|------|----------|----------|------|
| 1 | 列表页 | 首次加载 | `lastMessagePreview` (纯文本) | ❌ 未实现 | `SessionMeta` 缺少字段 |
| 2 | 列表页 | 实时更新 | `lastMessagePreview` (纯文本) | ❌ 未实现 | 通过 `daemon:newMessage` 更新 |
| 3 | 详情页 | 首次加载 | `contentBlocks` (结构化) | ✅ 正常 | vlaude-core FFI 已返回 |
| 4 | 详情页 | 实时推送 | `contentBlocks` (结构化) | ❌ 未实现 | 推送时 `contentBlocks: nil` |

---

## 2. 场景分析

### 2.1 列表页（SessionListView）

**用途**：快速浏览，识别会话内容

**数据流**：
```
getSessions → listSessionsLegacy → ai-cli-session-db FFI → SessionMeta
```

**当前问题**：
- iOS `Session` 模型有 `lastMessage: Message?` 字段
- 但 `SessionMeta` 从未返回此字段
- **功能从未实现**

**设计决策**：
- 列表页只需要纯文本摘要，不需要 contentBlocks
- 在 `SessionMeta` 中新增 `lastMessagePreview` 字段
- 简化格式：`"帮我重构这个函数"` / `"🔧 Bash: ls -la"` / `"💭 思考中..."`

### 2.2 详情页（SessionDetailView）

**用途**：完整查看消息，富文本渲染

**数据流（首次加载）**：
```
getSessionMessages → getMessagesLegacy → vlaude-core FFI → enrich_messages_with_content_blocks()
```

**数据流（实时推送）**：
```
FileWatcher → processNewMessages() → client.pushMessage() → daemon:newMessage → iOS
```

**当前问题**：
- 首次加载：✅ `enrich_messages_with_content_blocks()` 已返回 contentBlocks
- 实时推送：❌ `pushMessage()` 传入 `contentBlocks: nil`

**原因分析**：
- `processNewMessages()` 在处理 FileWatcher 检测到的新消息时
- 为了避免阻塞主线程，没有同步解析 contentBlocks
- 导致推送的消息没有结构化内容

---

## 3. 解决方案

### 3.1 统一事件设计

使用一个 `daemon:newMessage` 事件同时满足列表页和详情页的需求：

```typescript
// daemon:newMessage payload
{
  sessionId: String,
  message: {
    uuid: String,
    type: "user" | "assistant",
    timestamp: String,
    message: {
      role: String,
      content: String | Array  // 原始 content
    },
    // 新增字段
    contentBlocks: Array<ContentBlock>,  // 结构化内容（详情页用）
    preview: String                       // 纯文本预览（列表页用）
  },
  timestamp: String
}
```

**iOS 端处理**：
- 列表页：提取 `preview` 更新 lastMessage 显示
- 详情页：提取 `contentBlocks` 渲染富文本

### 3.2 分场景实现

#### 场景 1 & 2：列表页（加载 + 实时更新）

**改动清单**：

1. **ai-cli-session-db** (Rust)
   - `SessionMeta` 新增字段：
     ```rust
     pub struct SessionMeta {
         // ... 现有字段
         pub last_message_type: Option<String>,     // "user" | "assistant"
         pub last_message_preview: Option<String>,  // 纯文本预览（100字符）
         pub last_message_at: Option<i64>,          // 时间戳（毫秒）
     }
     ```
   - `list_sessions()` 生成预览

2. **VlaudeKit** (Swift)
   - `SessionMeta` 新增对应字段
   - FFI 解析适配

3. **iOS** (Vlaude)
   - `Session` 模型使用新字段
   - `SessionRow` 直接显示 `lastMessagePreview`

4. **实时更新**（场景 2）
   - `daemon:newMessage` 携带 `preview` 字段
   - iOS 收到后更新对应 session 的 lastMessage 显示

#### 场景 4：详情页实时推送

**改动清单**：

1. **VlaudeKit** (Swift)
   - `processNewMessages()` 中解析 contentBlocks
   - 或在 Rust 端解析后通过 FFI 返回

2. **方案选择**：

   **方案 A：Swift 端解析**（推荐）
   - 在 `processNewMessages()` 调用现有的 `ContentBlockParser.parseMessage()`
   - 传给 `pushMessage()` 的 `contentBlocks` 参数
   - 优点：复用现有代码
   - 缺点：在主线程解析可能影响性能

   **方案 B：Rust 端解析**
   - 在 session-reader 中解析 contentBlocks
   - 通过 FFI 返回
   - 优点：性能更好
   - 缺点：需要在 Rust 复现 ContentBlockParser 逻辑

---

## 4. 预览生成规则

### 4.1 生成逻辑

```rust
fn generate_preview(message: &ParsedMessage) -> String {
    match message.message_type {
        MessageType::User => {
            // 用户消息：直接截取 content_text
            truncate_chars(&message.content_text, 100)
        }
        MessageType::Assistant => {
            // 助手消息：解析 content 数组，生成摘要
            generate_assistant_preview(&message.raw)
        }
        _ => String::new()
    }
}

fn generate_assistant_preview(raw: &str) -> String {
    // 1. 解析 raw JSON
    // 2. 遍历 content 数组
    // 3. 遇到 text：累积文本
    // 4. 遇到 tool_use：生成 "🔧 {name}: {简化参数}"
    // 5. 遇到 thinking：记录但不输出
    // 6. 如果只有 thinking，返回 "💭 思考中..."
    // 7. 截取前 100 字符
}

/// 按 Unicode 字符截断，避免多字节字符被截断
fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}
```

### 4.2 Fallback 规则

| 情况 | 预览内容 |
|------|----------|
| 只有 thinking 块 | `💭 思考中...` |
| 只有 tool_use 块 | `🔧 {tool_name}: {简化参数}` |
| text + tool_use 混合 | 优先显示 text 部分 |
| 空消息 | `（空消息）` |

### 4.3 tool_use 简化规则

| 工具名 | 参数 | 预览 |
|--------|------|------|
| Bash | `command` | `🔧 Bash: {command前30字符}` |
| Read | `file_path` | `🔧 Read: {文件名}` |
| Write | `file_path` | `🔧 Write: {文件名}` |
| Edit | `file_path` | `🔧 Edit: {文件名}` |
| Glob | `pattern` | `🔧 Glob: {pattern}` |
| Grep | `pattern` | `🔧 Grep: {pattern}` |
| 其他 | - | `🔧 {name}` |

---

## 5. 实现进度

### Phase 1: 列表页首次加载（场景 1）

- [x] ai-cli-session-db 改动
  - [x] SessionMeta 结构体扩展 (`ai-cli-session-collector/src/domain/types.rs`)
  - [x] SessionMetaC FFI 扩展 (`ai-cli-session-db/src/ffi.rs`)
  - [x] list_sessions_with_preview 函数 (`ai-cli-session-db/src/reader.rs`)
  - [x] 预览生成逻辑（含 Unicode 截断、fallback 规则）
- [x] VlaudeKit 改动
  - [x] SessionMeta Swift 结构体扩展 (`SessionReader.swift`)
  - [x] FFI 解析适配（文件路径）
  - [x] FfiSessionInfo 扩展预览字段 (`VlaudeFfiBridge.swift`)
  - [x] toSessionMeta() 传递预览字段
  - [x] reportSessionMetadata() 包含预览字段 (`VlaudeClient.swift`)
- [x] iOS 改动
  - [x] Session 模型适配 (`Session.swift` V5 字段)
  - [x] SessionRow UI 适配 (`SessionListView.swift`)

> **注意**：当前数据库路径 (vlaude-core → SharedDb) 不返回预览字段，字段为 nil。
> 文件路径 (ai-cli-session-db FFI) 已支持预览字段。

### Phase 2: 实时推送完善（场景 2 + 场景 4 合并）

> 场景 2（列表页实时更新）和场景 4（详情页实时推送）都是修改 `pushMessage` 参数，合并实现。

- [x] VlaudeKit 改动
  - [x] ContentBlockParser 新增 generatePreview() 方法
  - [x] processNewMessages() 生成 preview（`VlaudePlugin.swift`）
  - [x] pushMessage() 新增 preview 参数（`VlaudeClient.swift`）
- [x] iOS 改动
  - [x] WebSocketMessage 新增 preview 字段（`WebSocketManager.swift`）
  - [x] SessionListViewModel 监听 messageNew 更新预览
  - [x] Session.withPreview() 方法更新预览数据

> **注意**：详情页的 contentBlocks 渲染保持懒加载模式，由 iOS 按需解析。

---

## 6. 相关文档

- [数据同步架构](../architecture/data-sync.md) - 整体数据流设计
- [Vlaude CLAUDE.md](../../vlaude/.claude/CLAUDE.md) - Daemon 架构说明
