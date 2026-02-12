# iOS Timeline 现状与待办清单

> 创建时间: 2026-02-09
> 状态: 进行中
> 关联 session: `ac26134e`(验证器) → `7c6ad1c8`(渲染修复) → `f41e0e14`(推送修复)

## 1. 背景

Vlaude iOS app 通过两条路径获取 Claude Code 会话数据：

```
REST 历史加载:  iOS → vlaude-server → Rust daemon → JSONL 文件 → 返回消息
WebSocket 推送: ETerm VlaudeKit 监听文件变化 → FFI → server → iOS
```

用户反馈核心问题：**打开 session 后，初始消息能加载，但后续实时消息不更新，退出重进才能看到新消息。**

调查过程中发现问题横跨三个层面：推送管道、渲染逻辑、状态判定。

## 2. 验证器（ac26134e）

### 2.1 定位：可执行的数据规格说明

验证器不仅仅是一个审计工具，它是整个下游系统设计的**基础设施**。核心理念：

> "验证器得搞明白数据结构，那么我们才能搞明白 memex 的记忆链路应该怎么写、查询怎么查；还有是 iOS 的时间线应该怎么画，用户看到的时候怎么样才是最清晰的显示 Claude 的流程"

验证器跑在真实数据上，输出完整的结构字典。这个字典是三个下游消费者的**权威参考**：

```
验证器跑真实数据 → 输出完整结构字典
                       │
            ┌──────────┼──────────┐
            ▼          ▼          ▼
       Memex 记忆链路  iOS Timeline  adapter 解析
       (语义索引)     (Turn 渲染)    (JSONL→结构化)
```

- **Memex**: 决定哪些字段需要索引、记忆链如何构建、查询如何执行
- **iOS Timeline**: 决定 Turn 边界如何划分、事件如何分组、状态如何判定
- **adapter**: 决定 JSONL 行如何解析为结构化数据

验证器发现的每一个数据结构特征（compaction、subagent、fork 等），都直接影响这三个消费者的设计。因此验证器的完备性直接决定了下游系统的正确性。

### 2.2 是什么

Rust 实现的 AI CLI Session 数据结构合规性验证器，位于 `ai-cli-session-collector/src/validate/`。

六层验证架构：
- **L0 Field**: 字段白名单 + 类型检查 + 值域检查
- **L1 Line**: 按 type 检查必选/条件字段组合
- **L2 Block**: requestId 分组验证（parentUuid 链、stop_reason 位置）
- **L3 Turn**: Turn 边界检测 + 生命周期验证
- **L4 Session**: parentUuid 树、fork 检测、compaction 链、subagent 关联
- **L5 Project**: sessions-index 一致性、tool-results 交叉引用

### 2.3 验证结果

在 **461K 行**真实数据（9333 session、2266 JSONL）上完成收敛：**0 未知字段**。

暴露了 6 个 iOS Timeline 渲染层问题（详见 §3）。

### 2.4 验证器位置

```
ai-cli-session-collector/
├── src/bin/validate.rs          # CLI 入口
└── src/validate/
    ├── framework.rs             # Level, Severity, Finding
    ├── report.rs                # 输出格式化
    └── claude/
        ├── schema.rs            # L0 白名单（~40 顶层字段）
        ├── line.rs              # L1 字段组合
        ├── block.rs             # L2 requestId 分组
        ├── turn.rs              # L3 Turn 结构
        ├── session.rs           # L4 Session 树
        └── project.rs           # L5 项目文件
```

运行方式：
```bash
cd ai-cli-session-collector
cargo run --bin validate --features cli -- --source claude
```

## 3. iOS 渲染修复（7c6ad1c8）

### 3.1 已完成

| Fix | 问题 | 状态 | 改动文件 |
|-----|------|------|---------|
| Fix 1: isCompactSummary 过滤 | compaction summary 穿透 TurnBuilder 创建 2170 个假 Turn | **已完成** | TurnBuilder.swift |
| Fix 2: image block 支持 | ContentBlock 无 image case，用户粘贴图片丢失 | **已完成** | Message.swift, TurnBuilder.swift, TurnCard.swift, MessageTransformer.swift |
| Fix 3: compaction 边界可视化 | compact_boundary 被过滤，看不到上下文压缩 | **已完成** | Turn.swift, TurnBuilder.swift, SessionTimelineView.swift |
| Fix 4: 中断 tool_use 标记 | tool_use 无 tool_result 时无标识 | **已完成** | TurnBuilder.swift, CompactToolRow.swift |

### 3.2 未完成

| Fix | 问题 | 状态 | 说明 |
|-----|------|------|------|
| Fix 5: loadMore 分页 | 向上翻页不工作 | **未开始** | 需实机调试 nextCursor 传递链路 |
| openTurn "处理中" | 最后一个 Turn 永远显示"处理中" | **调查中** | 见 §5 |
| ~~Fix 6: stopReason 位置~~ | 验证器 L2 警告 | **不需要** | 不影响 TurnBuilder |

### 3.3 编译状态

Fix 1-4 代码已写入但**未编译验证**（`7c6ad1c8` session 在写 `isLastTurnClosed` 时被中断）。需要 `dev-runner__start` 编译 iOS target 验证。

## 4. 推送管道修复（f41e0e14，本 session）

### 4.1 推送链路

```
Claude Code 写 JSONL
    ↓ (AICliKit 事件 / FileWatcher 2s debounce)
VlaudePlugin.readAndPushNewMessages (主线程)
    ↓ 读取新消息，UUID 去重
VlaudeClient.pushMessage
    ↓ try? socketBridge?.notifyNewMessage (FFI)
SocketClientBridge.notifyNewMessage
    ↓ queue.sync { block_on(async { emit }) }
Rust SocketClient → Server DaemonGateway
    ↓ eventEmitter.emit('app.notifyNewMessage')
Server AppGateway → sessionSubscriptions → iOS
    ↓ socket.emit('message:new')
iOS SessionDetailViewModel callback → UI 更新
```

### 4.2 发现的 Bug

**Bug 1: 游标在推送前更新（严重）**
- 位置：`VlaudePlugin.swift` readAndPushNewMessages
- 问题：`cursor.fileOffset = result.newOffset` 在 `processNewMessages()` 之前执行
- 后果：推送失败时游标已前进，消息永久丢失，不可重试
- 修复：**已完成** — 游标移到推送成功之后

**Bug 2: `try?` 静默吞错误（严重）**
- 位置：`VlaudeClient.swift:957` `pushMessage`
- 问题：`try? socketBridge?.notifyNewMessage(...)` 吞掉所有 FFI 错误
- 后果：推送失败零日志、零感知
- 修复：**已完成** — 改为 `do/catch` + logError，方法返回 Bool

**Bug 3: processNewMessages 无返回值**
- 位置：`VlaudePlugin.swift` processNewMessages
- 问题：无法知道推送是否全部成功
- 修复：**已完成** — 返回 Bool，追踪所有消息推送状态

### 4.3 之前已修复的问题

| 问题 | 状态 |
|------|------|
| WebSocket session:subscribe 重连后不恢复 | **已修复** — activeMessageSubscriptions 追踪 + restoreSubscriptions |
| displayText 缺失（Bash 工具空白） | **已修复** — ToolExecution +displayText fallback |
| dev-runner start 不自动重启 | **已修复** — 检测 already_running 自动 stop→rebuild→launch |
| CoreNetworkKit SocketIO 线程安全 | **已修复** — emitWithAck 包裹 handleQueue.async（待 push 到远端） |

### 4.4 编译状态

推送修复已编译部署（`./scripts/build.sh plugins`），ETerm 已重启加载。**未实机验证**。

## 5. openTurn "处理中"问题

### 5.1 现象

iOS 打开任何 session，最后一个 Turn 永远显示"处理中"状态，即使 Claude 已经回复完毕。

### 5.2 链路分析

`openTurn` 是 REST 加载消息时后端返回的标记，表示"最后一个 Turn 是否仍在进行中"：

```
iOS loadMessages → server getSessionMessages → Rust daemon handle_request_session_messages
    → is_user_text() Turn 边界扫描 → 返回 open_turn: bool
```

### 5.3 7c6ad1c8 的尝试

1. 修改了 Rust daemon-logic 的 `open_turn` 判断逻辑
2. 编译了 FFI (`build.sh socket`)
3. 重启 ETerm — **问题仍在**

### 5.4 可能的根因（7c6ad1c8 的最终分析）

1. `socket-client-ffi` 可能不依赖 `daemon-logic`，改动没生效
2. `open_turn` 可能在 NestJS server 端计算，不走 daemon
3. **最可靠方案**：iOS 端拿到所有消息后自己判断 Turn 是否关闭，不依赖后端

### 5.5 状态

**未解决**。需要先确认 `openTurn` 的实际计算路径，再决定在哪一层修。

## 6. Timeline V2 实机验证问题（2026-02-10）

V2 视觉改动已编译部署（折叠/展开双模式、统计栏、去 finalResponse），实测发现以下数据层问题：

### 6.1 Thinking 内容泄漏为明文 text

**现象**：`<thinking>` 内容和 `</thinking>` 闭合标签被当作普通 text 渲染，未进入 ThinkingEventView 折叠条。

**根因**：数据分类链路中 role 映射丢失类型信息，见 §6.4。

### 6.2 系统标签泄漏到 Timeline

**现象**：`[system:stop_hook_summary]` + JSON 和 `[system:turn_duration]` 被渲染为 AI text 事件。

**根因**：DB 中 System 类型消息（role=3）在 SharedDbBridge 中被映射为 "assistant"，穿透 TurnBuilder 的 type 过滤。见 §6.4。

### 6.3 Command message XML 穿透

**现象**：`/chat` 等斜杠命令的 XML 标签（`<command-message>`, `<command-name>`, `<command-args>`）被当成用户消息创建了 Turn。

**可能根因**：
- command 消息在 JSONL 中可能是 `type: "user"` 但内容是 XML 元数据
- TurnBuilder 的 `looksLikeCommandOutput()` 依赖 `message.content` 文本前缀检测，推送路径的 content 可能结构不同

### 6.4 根因：SharedDbBridge role 映射丢失类型信息

**数据链路**：
```
JSONL type 字段 → collector 写入 DB type 列 → FFI 返回 role 整数 → Swift SharedDbBridge → VlaudeKit convertToRawMessage → push 到 iOS
```

**FFI 层正确区分 4 种类型**（`ai-cli-session-db/src/ffi.rs:651`）：
```rust
let role = match m.r#type {
    MessageType::User => 0,
    MessageType::Assistant => 1,
    MessageType::Tool => 2,
    MessageType::System => 3,
};
```

**Swift 层丢失了类型信息**（`SharedDbBridge.swift:257`）：
```swift
let roleStr = m.role == 0 ? "human" : "assistant"
// System(3) 和 Tool(2) 都被当成 "assistant"
```

然后 `convertToRawMessage` 用 `msg.role == "human" ? 0 : 1` 进一步简化，System 消息以 `messageType=1`（assistant）推送到 iOS。

**已修复**（2026-02-10 编译通过）：

1. `SharedDbBridge.swift:257` — switch 映射 4 种 role（0→human, 1→assistant, 2→tool, 3→system）
2. `convertToRawMessage` — `guard msg.role == "human" || msg.role == "assistant"` 过滤 system/tool
3. `convertToRawMessage` — `json["isMeta"] == true → return nil` 过滤 command message
4. **content_full → raw JSON content** — 从 raw JSONL 提取原始 `message.content` 数组（JSON 格式），
   替代 content_full（FTS 格式: "[Thinking] xxx\n回复"），ContentBlockParser 能正确按 block 解析
5. **混合 block eventType** — thinking+text 消息不再设单一 eventType，iOS 通过 contentBlocks 解析
6. 游标死循环防护 — 全部消息被过滤时仍推进游标

### 6.5 推送延迟/批量到达

**现象**：持续观测的 iOS 端不实时收到消息，而是在某个触发节点（如用户发新消息）一股脑到达。退出重进（REST 加载）总是能看到最新数据。

**可能原因**：
- `collectInFlight` 互斥 + `collectPending` 覆盖式（非队列）→ 多次触发只保留最后一次
- `notifyFileChange` 同步 RPC 阻塞 → 后续触发排队
- AICliKit 的 `responseComplete` 事件未可靠触发
- FileWatcher debounce 2s 不够及时，或某些 session 未安装 watcher

**需要运行时日志定位**，不是看代码能确定的。

## 7. 待办优先级（2026-02-11 更新）

### 已完成 ✅

- [x] **数据分类修复（§6.1-6.3）** — SharedDbBridge 4 类型 role 映射, convertToRawMessage 过滤, content 修复, 游标防护（已编译，待运行验证）
- [x] **bufferMessage debounce → throttle** — 修复流式更新卡顿（`SessionDetailViewModel.swift`，`333fb989`）
- [x] **移除重复审批 UI** — TurnCard 的 pendingApprovalCard 移除，CompactToolRow 已有审批按钮（`333fb989`）
- [x] **Hook 事件录制系统** — claude_hook.sh 录制 + replay-hooks.sh 回放 + PreToolUse/PostToolUse hook 覆盖（`333fb989`）
- [x] **推送管道修复** — 游标先推后移、try? 改 do/catch、processNewMessages 返回 Bool（`f41e0e14`，已编译）
- [x] **三端 DIAG 观察工具** — scripts/observe-diag.sh，全链路 1-25ms 实时性正常

### P0: Permission Request toolUseId 关联

- [ ] Server 新增 `ToolUseCorrelator`（PreToolUse 缓存 → PermissionRequest enrichment）
- [ ] Server DaemonGateway 新增 `daemon:preToolUse` handler
- [ ] ETerm VlaudePlugin 转发 PreToolUse 到 Server
- [ ] iOS 用 toolUseId 精确匹配 pending approval
- [ ] 清理 unmatchedApprovals 临时方案
- 设计文档: [`docs/design/permission-request-correlator.md`](permission-request-correlator.md)

### P1: iOS 底部自动滚动

- [ ] 新消息到达时不能自动滚到底部（或滚动行为异常）
- [ ] 需要排查 SessionTimelineView / ScrollView 的 scrollTo 逻辑
- [ ] 考虑用户正在向上浏览历史时不应强制滚底

### P2: 解决 openTurn "处理中"

- [ ] 确认 `openTurn` 计算在哪一层（daemon Rust / NestJS server / iOS）
- [ ] 选择修复策略：后端修正 or iOS 端自行判断
- [ ] 实现并验证

### P3: loadMore 分页（向上翻页不工作）

- [ ] 加日志追踪 `nextCursor` 在 iOS 端的值
- [ ] 确认 `loadMessages(reset: false)` + `before` 参数传递链路
- [ ] 需要实机调试

### P4: 推送实时性（§6.5）

- [ ] 在 VlaudePlugin 各触发点加 log（responseComplete/promptSubmit/fileWatcher）
- [ ] 在 collectAndPushNewMessages 入口/出口加 log（collectInFlight 状态、消息数量）
- [ ] 重编译 → 复现 → 看日志定位卡点

### P5: Subagent 详情展示

- [ ] SubagentRow 当前只显示 agent 名称，不展示内部事件
- [ ] 需要设计子时间线展开交互

### P6: 其他 Timeline 体验

- [ ] MCP 工具交互体验优化
- [ ] "Request interrupted by User" 缺少友好交互提示
- [ ] 边界问题逐个排查

### P7: 技术债

- [ ] CoreNetworkKit SocketIO 线程安全修复 push 到 `vimo-ai/CoreNetworkKit` 远端
- [ ] V3 数据分类修复重启 ETerm 运行验证（§6.1-6.3 泄漏是否消失）
- [ ] VlaudeKit 入库时过滤 rollout session
- [ ] `~/.vimo` 数据清理 + 归档合并（见 MEMORY.md）
- [ ] Daemon 权限请求死代码激活 + 事件名 mismatch 修复（见 permission-request-correlator.md §6）

## 8. Session 关联记录

| Session | 日期 | 主要工作 |
|---------|------|---------|
| `ac26134e` | 02-09 | 验证器六层架构 |
| `7c6ad1c8` | 02-09 | iOS 渲染修复 Fix 1-4 |
| `f41e0e14` | 02-10 | 推送管道修复 + V3 数据分类修复 + DIAG 工具 |
| `333fb989` | 02-11 | iOS debounce→throttle + 审批 UI 去重 + Hook 录制系统 + ToolUseId Correlator 设计 |

## 9. 关键文件索引

### 推送链路（VlaudeKit → Server）
| 文件 | 说明 |
|------|------|
| `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/VlaudePlugin.swift` | 推送入口 readAndPushNewMessages |
| `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/VlaudeClient.swift` | pushMessage FFI 调用 |
| `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/SocketClientBridge.swift` | Swift→Rust FFI 桥接 |
| `vlaude/packages/vlaude-core/socket-client-ffi/src/lib.rs` | FFI 层 block_on |
| `vlaude/packages/vlaude-core/socket-client/src/client.rs` | Rust async emit |
| `vlaude/packages/vlaude-server/src/module/daemon-gateway/daemon.gateway.ts` | Server 接收 |
| `vlaude/packages/vlaude-server/src/gateway/app.gateway.ts` | Server 转发到 iOS |

### iOS 渲染层
| 文件 | 说明 |
|------|------|
| `vlaude/packages/Vlaude/Vlaude/Services/TurnBuilder.swift` | Turn 构建核心 |
| `vlaude/packages/Vlaude/Vlaude/Models/Message.swift` | ContentBlock 定义 |
| `vlaude/packages/Vlaude/Vlaude/Models/Turn.swift` | Turn 模型 |
| `vlaude/packages/Vlaude/Vlaude/Views/SessionTimelineView.swift` | Timeline 主视图 |
| `vlaude/packages/Vlaude/Vlaude/Views/TurnCard.swift` | Turn 卡片渲染 |
| `vlaude/packages/Vlaude/Vlaude/ViewModels/SessionDetailViewModel.swift` | 数据加载 + 实时监听 |
| `vlaude/packages/Vlaude/Vlaude/Services/VlaudeWebSocketClient.swift` | iOS WebSocket 客户端 |

### 验证器
| 文件 | 说明 |
|------|------|
| `ai-cli-session-collector/src/bin/validate.rs` | CLI 入口 |
| `ai-cli-session-collector/src/validate/claude/` | Claude 数据源规则 |

### 设计文档
| 文件 | 说明 |
|------|------|
| `docs/design/permission-request-correlator.md` | ToolUseId 关联方案（Server Correlator） |
| `docs/design/session-chain-linking.md` | 多源数据结构调研（验证器的权威参考） |
| `docs/design/agent-v2-rw-separation.md` | V2 读写分离架构 |
