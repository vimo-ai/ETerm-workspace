# ETerm 数据同步架构设计

## 1. 概述

ETerm 是一个三端系统，需要实现高效、实时的数据同步：

- **ETerm/Daemon**：macOS 端，本地运行，持有原始数据
- **Server**：云端/自托管，中转与缓存层
- **iOS (Vlaude)**：移动端，远程访问

### 1.1 当前问题

- iOS 每次打开 session 需要全量读取，延迟高
- 权限请求等实时事件可能丢失（iOS 后开场景）
- Server 是纯转发，无状态缓存

### 1.2 设计目标

- 快速响应：iOS 能快速看到数据
- 实时同步：数据变化能及时推送
- 可靠性：不漏消息，状态一致

---

## 2. 数据层级

```
┌─────────────────────────────────────────────────────────────┐
│                         JSONL                               │
│                      (唯一真相源)                            │
│                                                             │
│  Claude Code 直接写入，append-only，不可修改                 │
└─────────────────────────────────────────────────────────────┘
                           │
                      解析 (ai-cli-session-collector)
                           ↓
┌─────────────────────────────────────────────────────────────┐
│                      Daemon DB                              │
│                     (本地真相 #2)                            │
│                                                             │
│  SQLite，存储解析后的结构化数据                              │
│  位置：~/.vimo/db/ai-cli-session.db                         │
└─────────────────────────────────────────────────────────────┘
                           │
                      增量同步
                           ↓
┌─────────────────────────────────────────────────────────────┐
│                      Server DB                              │
│                     (线上副本)                               │
│                                                             │
│  与 Daemon DB 结构一致，通过推送保持同步                     │
└─────────────────────────────────────────────────────────────┘
```

---

## 3. 连接架构

### 3.1 两层连接

```
┌─────────┐              ┌─────────┐              ┌─────────────────┐
│   iOS   │ ←─────────→  │ Server  │ ←─────────→  │ ETerm/Daemon    │
└─────────┘              └─────────┘              └─────────────────┘
              连接 1                    连接 2
           (WebSocket)              (Socket.IO)
```

### 3.2 连接 1：iOS ↔ Server

| 方向 | 触发 | 内容 |
|------|------|------|
| iOS → Server | 进入场景 | 请求拉取数据 |
| Server → iOS | 收到 Daemon 推送 | 转发变化 |

### 3.3 连接 2：Server ↔ Daemon

| 方向 | 触发 | 内容 |
|------|------|------|
| Daemon → Server | 数据变化 | 推送增量（全局，不管 iOS 是否订阅） |
| Server → Daemon | iOS 请求 | 请求拉取最新数据 |

---

## 4. 核心原则

### 4.1 推拉结合

| 机制 | 特性 | 目的 |
|------|------|------|
| **订阅（推送）** | 全局、不变 | 保证不漏消息 |
| **主动拉取** | 场景触发 | 保证实时性 |

- **订阅是全局的**：Daemon 有变化就推给 Server，不随 iOS 页面切换变化
- **主动拉是场景触发的**：iOS 进入页面时主动拉取，确保当前数据最新

### 4.2 Server 响应拉取逻辑

iOS 请求数据时，Server **并行**执行：

```
iOS 请求数据
      │
      ▼
   Server
      │
      ├──→ 1. 读 Server DB（快，先返回给 iOS）
      │
      └──→ 2. 同时请求 Daemon（获取最新数据）
                  │
                  ▼
            3. 比对增量
                  │
                  ├──→ 有新数据：推送给 iOS
                  │
                  └──→ 更新 Server DB
```

**关键**：读 Server DB **且** 请求 Daemon，两者并行，缺一不可。
- 读 Server DB 保证快速响应
- 请求 Daemon 保证实时性

---

## 5. 推送链路

### 5.1 ETerm 模式（实时 + 全面）

```
Claude Code
    │
    ↓ Hook 回调
ClaudeKit (ClaudeSocketServer)
    │
    ↓ emit "claude.xxx"
VlaudeKit (VlaudePlugin)
    │
    ├──→ Hook 事件直接转发（极高实时性）
    │    - sessionStart, sessionEnd
    │    - permissionRequest, approvalAck
    │    - metricsUpdate
    │
    └──→ SessionWatcher 监听 JSONL（高实时性）
         - newMessage（新消息内容）
              │
              ↓
         VlaudeClient (Socket.IO)
              │
              ↓
           Server
              │
              ├──→ 写入 Server DB
              │
              └──→ 转发给 iOS
```

### 5.2 纯 Daemon 模式（仅 JSONL 监听）

```
Claude Code → JSONL
                │
                ↓ FSEvents 监听
             Daemon
                │
                ↓ 解析增量
           推送 Server
                │
                ├──→ 写入 Server DB
                │
                └──→ 转发给 iOS
```

### 5.3 推送事件类型

| 类别 | 事件 | 来源 |
|------|------|------|
| **连接** | online, offline, heartbeat | Daemon |
| **会话生命周期** | sessionStart, sessionEnd | Hook / JSONL |
| **实时消息** | newMessage | SessionWatcher / JSONL |
| **权限请求** | permissionRequest, permissionTimeout, approvalAck | Hook |
| **状态更新** | metricsUpdate, sessionUpdate | Hook |
| **列表变化** | sessionListUpdate, projectListUpdate | Daemon |
| **ETerm 状态** | etermOnline, etermOffline | VlaudeKit |

---

## 6. Server 职责

| 职责 | 触发 | 动作 |
|------|------|------|
| **接收推送** | Daemon 推送 | 1. 写入 Server DB（增量）<br>2. 转发给 iOS |
| **响应拉取** | iOS 请求 | 1. 读 Server DB（先返回）<br>2. 同时请求 Daemon<br>3. 有增量则推送 iOS + 更新 Server DB |
| **同步保障** | 持续 | 确保 Server DB 与 Daemon DB 一致 |

---

## 7. iOS 场景与数据需求

| 场景 | 数据需求 | 进入时动作 |
|------|----------|-----------|
| Project 列表 | Projects + 统计 | 主动拉取 Projects |
| Session 列表 | 某 Project 下的 Sessions | 主动拉取 Sessions |
| Session 详情 | 某 Session 的 Messages | 主动拉取 Messages |

**进入场景 = 主动拉取**，确保看到的数据是最新的。

---

## 8. 增量同步机制

### 8.1 正常流程

```
JSONL 新增
    ↓
Daemon DB 写入（基于 uuid 去重）
    ↓
推送 Server（新消息）
    ↓
Server DB 写入
    ↓
推送 iOS
```

### 8.2 特殊场景：tool_result 补全

Claude Code 的 tool 调用分两步：
1. 先写入 `tool_use`（发起调用）
2. 后追加 `tool_result`（调用结果）

**处理方式**：增量 + 最近 N 条比对
- 推送增量（新消息）
- 同时检查最近 N 条是否有状态变化
- 有变化则一并推送

### 8.3 同步锚点

基于 `sequence` + `uuid` 进行增量判断：
- `uuid`：消息唯一标识，用于去重
- `sequence`：消息序号，用于排序和增量判断
- 断线重连时，基于 `last_sequence` 请求增量

---

## 9. 权限请求持久化

### 9.1 问题

权限请求是短生命周期事件，但需要可靠传递：
- iOS 后开场景：权限请求已触发，但 iOS 还没打开
- Server 离线场景：ETerm 运行但 Server 没启动

### 9.2 方案：扩展 messages 表

```sql
-- Daemon DB messages 表新增字段
ALTER TABLE messages ADD COLUMN approval_status TEXT;
  -- 值: pending, approved, rejected, timeout, NULL(无需审批)

ALTER TABLE messages ADD COLUMN approval_resolved_at INTEGER;
  -- 审批完成时间戳（毫秒）
```

**实现状态**：✅ 已完成（Phase 1）
- 使用迁移系统（version-based migrations）
- ApprovalStatus 枚举：pending, approved, rejected, timeout
- 部分索引优化查询性能

### 9.3 UUID 映射关系（关键）

Claude JSONL 中，tool_use 消息的结构：

```json
{
  "uuid": "msg-abc123",          // 整个消息的 UUID
  "type": "assistant",
  "message": {
    "content": [
      {
        "type": "tool_use",
        "id": "tool-xyz789",      // tool_use block 的 ID
        "name": "Bash",
        "input": {...}
      }
    ]
  }
}
```

**关键发现**：
- `message.uuid` = 整个消息的 UUID（存储在 `messages.uuid`）
- `tool_use.id` = tool_use block 的 ID（存储在 `messages.tool_call_id`）
- Hook 传递的 `tool_use_id` = `tool_use.id`

**解决方案**：
- ai-cli-session-collector 提取 tool_use block 的 `id` 到 `tool_call_id` 字段
- DB 提供 `update_approval_status_by_tool_call_id` 方法
- VlaudeKit 使用 `tool_call_id` 而不是 `uuid` 更新审批状态

### 9.4 处理流程（实际实现）

```
1. Claude Code 触发权限请求
       │
       ↓ Hook (permission_request)
   ClaudeKit (ClaudeSocketServer)
       │
       ↓ emit "claude.permissionPrompt"
   VlaudeKit.handleClaudePermissionPrompt
       │
       ├──→ 存入 pendingApprovals 映射 (toolUseId → timestamp)
       │
       └──→ 推送给 iOS (尽力而为)

2. SessionWatcher 扫描到 tool_use 消息
       │
       ↓ didReceiveMessages
   VlaudeKit 检查 tool_call_id
       │
       ├──→ 如果在 pendingApprovals 中？
       │      │
       │      └──→ 立即标记为 pending
       │           dbBridge.updateApprovalStatusByToolCallId(
       │               toolCallId: toolCallId,
       │               status: .pending,
       │               resolvedAt: 0
       │           )
       │
       └──→ 推送消息给 Server/iOS

3. iOS 审批返回
       │
       ↓ permissionResponse (action: y/n/a)
   VlaudeKit.didReceivePermissionResponse
       │
       ├──→ 解析 action → ApprovalStatus
       │
       ├──→ 写回 DB (通过 tool_call_id)
       │    dbBridge.updateApprovalStatusByToolCallId(
       │        toolCallId: toolUseId,
       │        status: .approved/.rejected,
       │        resolvedAt: now
       │    )
       │
       └──→ 写入终端 (action + \r)
```

**关键设计**：
- **先写 DB 再推送**：在 SessionWatcher 检测到消息后立即标记为 pending
- **幂等性**：基于 tool_call_id 更新，重复调用不会产生副作用
- **错误容忍**：DB 写入失败不影响终端操作
- **返回更新计数**：检测更新是否成功（count > 0）

### 9.5 iOS 获取 pending 权限请求

```
iOS 打开 Session 详情
       │
       ├──→ 拉取 Messages（包含 approval_status, approval_resolved_at）
       │
       └──→ 过滤 approval_status = 'pending' 的消息
                  │
                  └──→ 显示审批按钮
```

**实现状态**：⏳ 待实现（Phase 4）
- Repository 层读取 approval_status 字段
- UI 层根据状态显示审批按钮

### 9.6 API 设计

**DB 层** (`claude-session-db/src/db.rs`)：
```rust
// 获取待审批消息
pub fn get_pending_approvals(&self, session_id: &str) -> Result<Vec<Message>>

// 通过 tool_call_id 更新审批状态（返回更新行数）
pub fn update_approval_status_by_tool_call_id(
    &self,
    tool_call_id: &str,
    status: ApprovalStatus,
    resolved_at: i64,
) -> Result<usize>

// 统计待审批数量
pub fn count_pending_approvals(&self, session_id: &str) -> Result<i64>
```

**Swift 层** (`VlaudeKit/SharedDbBridge.swift`)：
```swift
// 获取待审批消息
func getPendingApprovals(sessionId: String) throws -> [SharedMessage]

// 更新审批状态（返回更新行数）
func updateApprovalStatusByToolCallId(
    toolCallId: String,
    status: ApprovalStatus,
    resolvedAt: Int64
) throws -> Int
```

### 9.7 唯一丢失场景

**SessionWatcher 扫描之前 ETerm 崩溃**

概率极低，因为：
- Hook 触发和 JSONL 写入几乎同时完成
- SessionWatcher 使用 FSEvents，延迟极低（< 100ms）

---

## 10. 部署模式

支持两种模式，用户可选：

| 模式 | Server DB | 适用场景 | 特点 |
|------|-----------|----------|------|
| **纯转发** | 无（0 缓存） | 隐私敏感用户 | Server 不存储任何数据，所有请求穿透到 Daemon |
| **线上同步** | 有 | 便利优先用户 | Server 缓存数据，iOS 可快速访问 |

### 10.1 纯转发模式详解

- Server **完全不存储**任何业务数据
- 所有 iOS 请求都穿透到 Daemon
- 推送也是直接转发，不落盘
- 适用于对数据隐私要求极高的用户

---

## 11. 代码设计原则

### 11.1 接口统一，实现分离

```swift
// 对外接口统一
protocol SessionRepository {
    func getProjects() async -> [Project]
    func getSessions(projectId: Int) async -> [Session]
    func getMessages(sessionId: String) async -> [Message]
}

// 内部实现可切换
// - 纯转发模式：所有请求穿透到 Daemon
// - 线上同步模式：先读 Server DB，同时请求 Daemon
```

### 11.2 分层架构

```
┌─────────────────────────────────────────────────────────────┐
│  View Layer         │ 展示层，不关心数据来源                 │
├─────────────────────────────────────────────────────────────┤
│  ViewModel          │ 状态管理，调用 Repository              │
├─────────────────────────────────────────────────────────────┤
│  Repository         │ 数据获取策略（推拉结合）               │
├─────────────────────────────────────────────────────────────┤
│  DataSource         │ 单一数据源访问（Server/Daemon）        │
├─────────────────────────────────────────────────────────────┤
│  Transport          │ 网络层（HTTP/WebSocket）               │
└─────────────────────────────────────────────────────────────┘
```

### 11.3 协议清晰

- 每条消息有 `uuid`（唯一标识）
- 增量请求基于 `after_uuid`
- 推送和拉取使用相同的数据格式

---

## 12. 实现进度

### 12.1 Phase 1: DB 改动 ✅

- [x] Daemon DB: messages 表增加 `approval_status`, `approval_resolved_at` 字段
- [x] 迁移系统（version-based migrations）
- [x] ApprovalStatus 枚举（pending, approved, rejected, timeout）
- [x] 部分索引优化
- [x] API: `get_pending_approvals`, `update_approval_status_by_tool_call_id`
- [x] FFI 层扩展（MessageC, MessageInputC）

### 12.2 Phase 2: VlaudeKit 改动 ✅

- [x] ai-cli-session-collector: 提取 tool_use block 的 id 到 tool_call_id
- [x] SharedDbBridge: getPendingApprovals, updateApprovalStatusByToolCallId
- [x] VlaudePlugin: 实现"先写 DB 再推送"原则
- [x] pendingApprovals 映射机制
- [x] SessionWatcher 集成（检测并标记 pending）
- [x] 审批结果写回 DB（通过 tool_call_id）

### 12.3 Phase 3: Server 改动 ✅

- [x] 支持线上同步模式（写入 Server DB）
- [x] 支持纯转发模式（0 缓存）
- [x] iOS 拉取时并行读 DB + 请求 Daemon
- [x] 增量同步（Daemon → Server）
- [x] 连接/断连状态处理

**实现详情**：

1. **环境变量配置** (`DATA_SYNC_MODE`)：
   - `forward`：纯转发模式（默认），所有请求穿透到 Daemon
   - `sync`：线上同步模式，Server 缓存数据

2. **Prisma Schema 扩展** (`prisma/schema.prisma`)：
   - Message 表新增：`uuid`, `toolCallId`, `approvalStatus`, `approvalResolvedAt`
   - 唯一约束：`@@unique([uuid])`（用于去重）
   - 索引：`@@index([toolCallId])`, `@@index([approvalStatus])`

3. **DataSyncModule** (`src/module/data-sync/`)：
   - `DataSyncService`：封装模式切换逻辑
   - `ensureProject/ensureSession`：upsert 保证存在
   - `syncMessages/writeMessage`：基于 uuid 去重写入
   - `updateApprovalStatusByToolCallId`：更新审批状态

4. **SessionService 改动** (`src/module/session/session.service.ts`)：
   - `getSessionMessages`：根据模式切换行为
     - forward：直接透传 Daemon
     - sync：并行读 DB + 请求 Daemon，后台同步增量

5. **DaemonGateway 改动** (`src/module/daemon-gateway/daemon.gateway.ts`)：
   - `handleNewMessage`：sync 模式下写入 Server DB 再转发

6. **Codex 评估与修复**：
   - ✅ 修复 sync 模式响应策略：改为"先读 DB 立即返回，后台刷新"
   - ✅ 修复 `writeMessage` P2002 冲突处理：视为 benign
   - ✅ 修复 `syncMessages` 计数逻辑：区分 inserted/updated/skipped
   - ❌ 驳回 UUID 全局唯一性担忧：Claude UUID 本身是全局唯一的
   - ❌ 驳回 toolCallId 全局唯一性担忧：同上

7. **测试覆盖** (`src/test/unit/data-sync.spec.ts`)：
   - 14 个测试用例，覆盖 forward/sync 两种模式

### 12.4 Phase 4: iOS 改动 ✅

- [x] Message.swift 扩展：新增 `toolCallId`, `approvalStatus`, `approvalResolvedAt` 字段
- [x] MessageTransformer 改造：从 Messages 读取 approvalStatus 并设置 ToolExecution 状态
- [x] SessionDetailViewModel 优化：applyPendingApprovals 同时处理缺少 requestId 的情况
- [x] 显示待审批按钮（status = pending → .awaitingPermission）
- [x] 保留 pendingApprovals 用于存储 requestId 和处理时序问题

**实现详情**：

1. **Message.swift 扩展**：
   - 新增 `toolCallId: String?`：tool_use block 的 ID
   - 新增 `approvalStatus: String?`：pending, approved, rejected, timeout
   - 新增 `approvalResolvedAt: Int?`：审批完成时间戳（毫秒）
   - CodingKeys 扩展支持 JSON 解码

2. **MessageTransformer 改造** (`MessageTransformer.swift`)：
   - 新增 `approvalCache: [String: ApprovalInfo]`：缓存 toolCallId → 审批状态映射
   - `updateApprovalCache(from:)`：从 Messages 构建审批状态缓存
   - `parseApprovalStatus(_:)`：将字符串状态转换为 ToolApprovalStatus 枚举
   - `getApprovalStatus(for:)`：获取工具的审批状态
   - 所有 ToolExecution 创建点都使用 approvalCache 设置初始状态

3. **SessionDetailViewModel 优化** (`SessionDetailViewModel.swift`)：
   - `applyPendingApprovals()` 增强：
     - 原有逻辑：状态为 .none 时设置状态和 requestId
     - 新增逻辑：状态为 .awaitingPermission 但缺少 requestId 时，只设置 requestId
   - 保留 `pendingApprovals` 用于：
     - 存储 requestId（发送审批响应时需要）
     - 处理 WebSocket 推送先于 Messages API 返回的时序问题

4. **状态映射**：
   - Server `"pending"` → iOS `.awaitingPermission`
   - Server `"approved"` → iOS `.completed`
   - Server `"rejected"` → iOS `.rejected`
   - Server `"timeout"` → iOS `.timeout`

5. **数据流**：
   ```
   iOS 打开 Session 详情
         │
         ↓ 调用 API
   Server 返回 Messages（包含 approvalStatus）
         │
         ↓
   MessageTransformer.transform()
         │
         ├──→ updateApprovalCache() 构建状态缓存
         │
         └──→ 创建 ToolExecution 时应用状态
                  │
                  ↓
   UI 根据 ToolExecution.approvalStatus 显示按钮
   ```

6. **实时更新流程**：
   ```
   WebSocket 推送 ApprovalRequest
         │
         ↓
   存储到 pendingApprovals（包含 requestId）
         │
         ↓
   applyPendingApprovals()
         │
         ├──→ 状态为 .none：设置状态 + requestId
         │
         └──→ 状态为 .awaitingPermission 缺少 requestId：只设置 requestId
   ```
