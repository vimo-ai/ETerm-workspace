# VlaudeKit 实时同步方案

> 讨论日期：2025-12-31
> 状态：**Phase 1-3 已实现** ✅

## 0. 核心理解：Writer 协调机制

### 0.1 四组件共享写入

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    claude-session-db Writer 协调                             │
│                    ~/.vimo/db/claude-session.db                              │
└─────────────────────────────────────────────────────────────────────────────┘

                    ┌──────────────────────────────────────┐
                    │          Writer 优先级                │
                    │                                      │
                    │  优先级 2 (高): MemexKit, VlaudeKit  │  ← ETerm 插件
                    │  优先级 1 (低): memex-rs, vlaude-daemon │  ← 后台 daemon
                    │                                      │
                    │  规则:                               │
                    │  - 高优先级可抢占低优先级             │
                    │  - 同优先级先到先得                   │
                    │  - 心跳 10s，超时 30s 后可接管        │
                    └──────────────────────────────────────┘
```

### 0.2 七种运行场景

| # | ETerm | Memex | Vlaude | Writer | 说明 |
|---|:-----:|:-----:|:------:|--------|------|
| 1 | ✅ | - | - | 插件 | 日常开发，ETerm 打开 |
| 2 | - | ✅ | - | memex-rs | 纯搜索场景 |
| 3 | - | - | ✅ | daemon | 手机远程访问，Mac 只跑 daemon |
| 4 | ✅ | ✅ | - | 插件 | ETerm + Memex 服务 |
| 5 | ✅ | - | ✅ | 插件 | ETerm + 远程同步 |
| 6 | - | ✅ | ✅ | 先到先得 | 无 ETerm，daemon 们竞争 |
| 7 | ✅ | ✅ | ✅ | 插件 | 全家桶 |

**核心规律：当 ETerm 运行时，插件始终是 Writer（优先级高）**

### 0.3 各组件触发方式对比

| 组件 | 触发方式 | 监听范围 | 上下文信息 | 场景 |
|------|----------|----------|------------|------|
| **VlaudeKit** | Hooks + 文件监听 | 精确监听活跃 session 文件 | terminalId, sessionId, cwd | ETerm 内 Claude |
| **MemexKit** | Hooks + 文件监听 | 精确监听活跃 session 文件 | terminalId, sessionId, cwd | ETerm 内 Claude |
| **vlaude-daemon** | 递归文件监听 | 监听整个 `~/.claude/projects/` | 只有文件路径 | 所有 Claude |
| **memex-rs** | 递归文件监听 | 监听整个 `~/.claude/projects/` | 只有文件路径 | 所有 Claude |

**VlaudeKit vs vlaude-daemon 核心差异**：

```
VlaudeKit 拿到的信息 (Hooks 提供):
┌─────────────────────────────────────────┐
│ terminalId: 42        ← VlaudeKit 独有  │
│ sessionId: "abc123"                     │
│ transcriptPath: "/.../abc123.jsonl"     │
│ cwd: "/Users/xxx/ETerm"  ← 精确        │
│ event_type: "session_start"             │
└─────────────────────────────────────────┘

vlaude-daemon 拿到的信息 (FSEvents):
┌─────────────────────────────────────────┐
│ path: "/.../abc123.jsonl"               │
│ kind: Create/Modify                     │
│                                         │
│ ❌ 不知道 terminalId                     │
│ ⚠️ cwd 需要读文件解析                    │
│ ⚠️ session 生命周期需要推断              │
└─────────────────────────────────────────┘
```

**两者定位**：
- **VlaudeKit**：ETerm 内 Claude 的精确实时同步（上下文丰富，Hooks 驱动）
- **vlaude-daemon**：ETerm 外 Claude 的通用实时同步（覆盖更广，文件监听驱动）

### 0.4 设计边界（已确认）

```
┌─────────────────────────────────────────────────────────────────┐
│                        覆盖范围边界                              │
└─────────────────────────────────────────────────────────────────┘

✅ 覆盖场景：
   - ETerm 内使用 Claude → VlaudeKit/MemexKit 通过 hooks 精确触发
   - ETerm 关闭，daemon 运行 → daemon 通过文件监听触发

❌ 不覆盖场景（设计边界）：
   - ETerm 打开，但用户在外部（终端/VS Code）使用 Claude
   - 这种情况插件无法感知，**不做处理**

📌 用户指引：
   - 如需索引外部 Claude 活动，应关闭 ETerm 并运行 daemon
```

---

## 1. 背景

### 1.1 当前架构概览

```
[iOS App]                    [云端服务]                    [本地]
Vlaude/                      vlaude-server/               ETerm + VlaudeKit

┌──────────────────┐         ┌──────────────────┐         ┌──────────────────┐
│ ProjectListVM    │  HTTP   │ ProjectController│  SQL    │ ai-cli-session.db│
│ loadProjects()   │ ──────► │ SharedDbService  │ ──────► │ (SQLite)         │
└──────────────────┘         └──────────────────┘         └──────────────────┘
        │                           │
        │ WebSocket                 │ WebSocket /daemon
        ▼                           ▼
┌──────────────────┐         ┌──────────────────┐         ┌──────────────────┐
│ WebSocketManager │ ◄────── │ AppGateway       │ ◄────── │ VlaudeKit        │
│                  │         │ DaemonGateway    │         │ VlaudeClient     │
└──────────────────┘         └──────────────────┘         └──────────────────┘
```

### 1.2 组件位置

| 组件 | 路径 | 职责 |
|------|------|------|
| VlaudeKit | `english/Plugins/VlaudeKit/` | ETerm 插件，连接 vlaude-server |
| ClaudeKit | `english/Plugins/ClaudeKit/` | Claude hooks 监听，广播事件 |
| vlaude-server | `claude/packages/vlaude-server/` | 云端服务，iOS 与 ETerm 中转 |
| Vlaude iOS | `claude/packages/Vlaude/` | iOS 客户端 |
| vlaude-daemon (NestJS) | `claude/packages/vlaude-daemon/` | **已过期**，仅作参考 |

### 1.3 关键数据路径

- Claude 会话文件：`~/.claude/projects/{encodedPath}/{sessionId}.jsonl`
- 共享数据库：`~/.vimo/db/ai-cli-session.db`

---

## 2. 已确认的链路状态

### 2.1 ✅ ETerm 状态通知 - 完整连通

```
VlaudeKit                    DaemonGateway                AppGateway                  iOS
───────────────────────────────────────────────────────────────────────────────────────────
daemon:etermOnline        → handleEtermOnline()        → app.etermStatusChanged    → eterm:statusChanged
daemon:etermSessionAvail  → handleEtermSessionAvail()  → app.etermSessionAvailable → eterm:sessionAvail
daemon:etermSessionUnavail→ handleEtermSessionUnavail()→ app.etermSessionUnavail   → eterm:sessionUnavail
```

**验证点**：
- VlaudeClient.swift:350 `emit("daemon:etermOnline")`
- VlaudeClient.swift:381 `emit("daemon:etermSessionAvailable")`
- DaemonGateway.ts:946 `@SubscribeMessage('daemon:etermOnline')`
- WebSocketManager.swift:248 `socket.on("eterm:statusChanged")`

### 2.2 ⚠️ 项目更新通知 - 缺少触发源

```
[???]                        DaemonGateway                AppGateway                  iOS
───────────────────────────────────────────────────────────────────────────────────────────
daemon:projectUpdate      → handleProjectUpdate()      → app.notifyProjectUpdate   → project:updated
```

**问题**：VlaudeKit 没有发送 `daemon:projectUpdate` 事件

### 2.3 ⚠️ 消息实时推送 - 推送时机不精确

**当前实现** (VlaudePlugin.swift)：
```swift
case "claude.responseComplete":
    // 只在 Claude 回复完成后推送
    client?.pushNewMessages(sessionId: sessionId, transcriptPath: transcriptPath)
```

**问题**：
- 用户发送消息时不推送
- 只有 Claude 回复后才推送（可能包含多条消息）
- iOS 端看到"跳跃"式更新

---

## 3. 最终方案：Hooks + 增量监听（方案 C）

> ✅ 已确认：这是实时性最强的方案

### 3.1 设计目标

1. **极致实时性**：文件写入 → 立即感知 → 立即推送
2. **完整性**：用户消息、Claude 消息、工具调用都推送
3. **顺序性**：严格按消息顺序推送
4. **资源效率**：只监听活跃 session 文件，增量解析

### 3.2 为什么方案 C 最强

| 方案 | 推送时机 | 实时性 | 问题 |
|------|----------|--------|------|
| A: promptSubmit + responseComplete | Hooks 触发时 | 中等 | 错过中间消息 |
| B: 只在 responseComplete | Claude 完成后 | 低 | 用户消息延迟 |
| **C: Hooks + 增量监听** | **文件任何写入** | **极高** | 无 |

**方案 C 的优势**：
- Hooks 提供精确触发点（知道何时开始/结束监听）
- 文件监听提供极致实时性（任何写入都感知）
- 只监听活跃文件，资源开销低

### 3.3 架构设计

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    VlaudeKit 增量监听方案                                    │
└─────────────────────────────────────────────────────────────────────────────┘

claude.sessionStart
    │
    │  拿到: terminalId, sessionId, transcriptPath, cwd
    │
    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  1. 通知 Server: session 上线                                                │
│     daemon:etermSessionAvailable { sessionId, projectPath, terminalId }     │
│                                                                              │
│  2. 索引到 SharedDb                                                          │
│     upsertProject(cwd) → upsertSession(sessionId)                           │
│                                                                              │
│  3. 开始监听 transcriptPath                                                  │
│     SessionWatcher.startWatching(sessionId, transcriptPath)                 │
│     使用 DispatchSource 监听 .write 事件                                     │
└─────────────────────────────────────────────────────────────────────────────┘
    │
    │  文件有写入（Claude 写入任何消息）
    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  DispatchSource .write 触发                                                  │
│                                                                              │
│  1. 增量读取: lastOffset → EOF                                               │
│  2. 解析 JSONL 行                                                            │
│  3. 推送每条消息: daemon:newMessage                                          │
│  4. 更新 lastOffset                                                          │
│  5. 索引消息到 SharedDb (Writer 时)                                          │
└─────────────────────────────────────────────────────────────────────────────┘
    │
    │  文件继续写入...（循环）
    │
    ▼
claude.sessionEnd
    │
    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  1. 通知 Server: session 下线                                                │
│     daemon:etermSessionUnavailable { sessionId, projectPath }               │
│                                                                              │
│  2. 停止监听                                                                 │
│     SessionWatcher.stopWatching(sessionId)                                  │
│     释放 DispatchSource                                                      │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 3.3 核心组件

#### SessionWatcher

```swift
// VlaudeKit/Sources/VlaudeKit/SessionWatcher.swift

/// 单个会话的监听状态
struct WatchedSession {
    let sessionId: String
    let terminalId: Int
    let transcriptPath: String
    var lastOffset: UInt64           // 上次读取的文件位置
    var lastMessageUUID: String?     // 上次推送的消息 UUID
    var dispatchSource: DispatchSourceFileSystemObject?
}

/// 会话文件监听器
final class SessionWatcher {
    private var sessions: [String: WatchedSession] = [:]  // sessionId -> WatchedSession
    private let queue = DispatchQueue(label: "com.eterm.vlaude.watcher")

    weak var delegate: SessionWatcherDelegate?

    /// 开始监听会话
    func startWatching(sessionId: String, terminalId: Int, transcriptPath: String)

    /// 停止监听会话
    func stopWatching(sessionId: String)

    /// 停止监听终端的所有会话
    func stopWatchingTerminal(terminalId: Int)

    /// 停止所有监听
    func stopAll()
}

protocol SessionWatcherDelegate: AnyObject {
    /// 有新消息需要推送
    func sessionWatcher(_ watcher: SessionWatcher, didReceiveMessages messages: [RawMessage], for sessionId: String)
}
```

#### 增量解析逻辑

```swift
/// 增量读取新消息
private func readNewMessages(session: inout WatchedSession) -> [RawMessage] {
    guard let handle = FileHandle(forReadingAtPath: session.transcriptPath) else {
        return []
    }
    defer { try? handle.close() }

    // 跳到上次读取位置
    try? handle.seek(toOffset: session.lastOffset)

    // 读取新内容
    guard let newData = try? handle.readToEnd(), !newData.isEmpty else {
        return []
    }

    // 更新偏移量
    session.lastOffset += UInt64(newData.count)

    // 解析 JSONL（每行一条消息）
    let lines = String(data: newData, encoding: .utf8)?
        .components(separatedBy: .newlines)
        .filter { !$0.isEmpty } ?? []

    return lines.compactMap { line in
        // 使用 SessionReader FFI 解析
        sessionReader.parseMessage(line)
    }
}
```

### 3.4 VlaudePlugin 集成

```swift
// VlaudePlugin.swift 修改

public final class VlaudePlugin: NSObject, Plugin {
    private var sessionWatcher: SessionWatcher?

    public func activate(host: HostBridge) {
        // ... 现有代码 ...

        // 初始化 SessionWatcher
        sessionWatcher = SessionWatcher()
        sessionWatcher?.delegate = self
    }

    public func handleEvent(_ eventName: String, payload: [String: Any]) {
        switch eventName {
        case "claude.sessionStart":
            // 新增：开始监听
            if let sessionId = payload["sessionId"] as? String,
               let terminalId = payload["terminalId"] as? Int,
               let transcriptPath = payload["transcriptPath"] as? String {
                sessionWatcher?.startWatching(
                    sessionId: sessionId,
                    terminalId: terminalId,
                    transcriptPath: transcriptPath
                )
            }

        case "claude.sessionEnd":
            // 新增：停止监听
            if let sessionId = payload["sessionId"] as? String {
                sessionWatcher?.stopWatching(sessionId: sessionId)
            }

        case "core.terminal.didClose":
            // 新增：终端关闭时停止监听
            if let terminalId = payload["terminalId"] as? Int {
                sessionWatcher?.stopWatchingTerminal(terminalId: terminalId)
            }

        // ... 其他事件 ...
        }
    }
}

// MARK: - SessionWatcherDelegate
extension VlaudePlugin: SessionWatcherDelegate {
    func sessionWatcher(_ watcher: SessionWatcher, didReceiveMessages messages: [RawMessage], for sessionId: String) {
        for message in messages {
            client?.pushMessage(sessionId: sessionId, message: message)
        }
    }
}
```

### 3.5 移除的代码

```swift
// VlaudePlugin.swift - 移除

case "claude.responseComplete":
    // 移除：不再在这里推送消息
    // client?.pushNewMessages(sessionId: sessionId, transcriptPath: transcriptPath)

    // 保留：索引到 SharedDb
    if let transcriptPath = payload["transcriptPath"] as? String {
        client?.indexSession(path: transcriptPath)
    }
```

---

## 4. 实现计划

### Phase 1：SessionWatcher 基础实现 ✅ (2025-12-31 完成)
- [x] 创建 `SessionWatcher.swift`
- [x] 实现 DispatchSource 文件监听
- [x] 实现增量解析逻辑（lastOffset）
- [x] 实现消息去重（lastMessageUUID）
- [ ] 单元测试

### Phase 2：VlaudePlugin 集成 ✅ (2025-12-31 完成)
- [x] 集成 SessionWatcher
- [x] `claude.sessionStart` 时开始监听（新增事件处理）
- [x] `claude.sessionEnd` 时停止监听
- [x] 修改 `responseComplete` 推送逻辑（由 SessionWatcher 接管）
- [ ] 集成测试

### Phase 3：完善通知链路 ✅ (2025-12-31 完成)
- [x] `session_start` 时发送 `daemon:projectUpdate`
- [x] 添加 `reportProjectUpdate()` 方法
- [ ] iOS 端项目列表刷新验证

### Phase 4：服务端优化（可选）
- [ ] DaemonGateway 增加消息去重
- [ ] AppGateway 增加消息缓存
- [ ] iOS 端消息顺序校验

---

## 5. 已确认的设计决策

### 5.1 ✅ 使用 DispatchSource（已确认）

**选择 DispatchSource (GCD)**：
- Swift 原生支持
- 代码更简洁
- 适合 VlaudeKit 作为 Swift 插件

### 5.2 ✅ 消息去重策略（已确认）

**方案**：
- 记录 `lastMessageUUID`
- 推送前检查是否已推送
- 避免 DispatchSource 频繁触发导致重复推送

### 5.3 ✅ project:updated 事件触发（已确认）

**触发点**：
- `session_start` 时检查项目是否为新项目，如果是则发送 `daemon:projectUpdate`
- 这样 iOS 端可以刷新项目列表

### 5.4 ✅ 设计边界（已确认）

- VlaudeKit 只负责 ETerm 内的 Claude 活动
- ETerm 打开时外部 Claude 活动不索引
- 如需覆盖外部 Claude，应关闭 ETerm 并运行 daemon

---

## 6. 参考资料

### 6.1 vlaude-daemon NestJS 实现（已过期，仅参考）

- 位置：`claude/packages/vlaude-daemon/src/module/file-watcher/`
- 使用 chokidar 监听文件变化
- 增量解析逻辑可参考

### 6.2 相关文件

| 文件 | 说明 |
|------|------|
| `VlaudePlugin.swift` | 主插件，事件处理 |
| `VlaudeClient.swift` | WebSocket 客户端 |
| `ClaudePlugin.swift` | Claude hooks 处理 |
| `SessionReader.swift` | jsonl 解析 FFI |
| `DaemonGateway.ts` | Server 端 Daemon 网关 |
| `AppGateway.ts` | Server 端 iOS 网关 |

---

## 更新日志

| 日期 | 内容 |
|------|------|
| 2025-12-31 | **实现完成**：Phase 1-3 全部实现 |
| 2025-12-31 | 新增 `SessionWatcher.swift` - DispatchSource 文件监听 + 增量解析 |
| 2025-12-31 | 新增 `claude.sessionStart` 事件处理，提前建立映射和开始监听 |
| 2025-12-31 | 新增 `reportProjectUpdate()` 和 `pushMessage()` 方法 |
| 2025-12-31 | 确认最终方案 C（Hooks + 增量监听），更新设计决策 |
| 2025-12-31 | 更正 VlaudeKit vs daemon 对比（daemon 是递归监听，不是扫描） |
| 2025-12-31 | 添加 Writer 协调机制和设计边界说明 |
| 2025-12-31 | 初始版本，确认架构和方案方向 |
