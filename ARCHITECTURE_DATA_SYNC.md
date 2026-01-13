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
  -- 审批完成时间戳
```

### 9.3 处理流程

```
Claude Code 触发权限请求
       │
       ↓ Hook
   VlaudeKit
       │
       ├──→ 1. 写入 Daemon DB (approval_status = 'pending')
       │         ↑ 本地操作，必成功
       │
       └──→ 2. 推送 Server（尽力而为）
                  │
                  ↓ (如果 Server 在线)
              Server
                  │
                  ├──→ 写入 Server DB
                  │
                  └──→ 转发 iOS
```

**关键**：先写 DB，再推送。即使 Server 不在线，数据也已持久化。

### 9.4 iOS 获取 pending 权限请求

```
iOS 打开 Session 详情
       │
       ├──→ 拉取 Messages（包含 approval_status）
       │
       └──→ 找出 approval_status = 'pending' 的消息
                  │
                  └──→ 显示审批按钮
```

### 9.5 唯一丢失场景

**写入 DB 之前 ETerm 崩溃**

概率极低，本地写入几乎瞬时完成。

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

## 12. 下一步

### 12.1 DB 改动

- [ ] Daemon DB: messages 表增加 `approval_status`, `approval_resolved_at` 字段
- [ ] Server DB: schema 与 Daemon DB 保持一致
- [ ] 迁移脚本

### 12.2 VlaudeKit 改动

- [ ] 权限请求时先写 DB 再推送
- [ ] 审批结果更新 DB

### 12.3 Server 改动

- [ ] 支持线上同步模式（写入 Server DB）
- [ ] 支持纯转发模式（0 缓存）
- [ ] iOS 拉取时并行读 DB + 请求 Daemon

### 12.4 iOS 改动

- [ ] Repository 层重构（推拉结合）
- [ ] 从 Messages 数据中读取 approval_status
- [ ] 处理"先旧后新"的增量合并
