# ai-cli-session-db Agent 架构设计

> 创建时间: 2026-01-23
> 状态: ✅ 已完成（Phase 5 - Agent 架构迁移完成）

## 1. 背景与问题

### 1.1 现有架构

```
4 个组件共享 SQLite 数据库：
├── VlaudeKit (优先级 3)  - approve 写入
├── MemexKit (优先级 2)   - 通过 HTTP 调用 memex-rs
├── vlaude-daemon (优先级 1) - approve + collection
└── memex-rs (优先级 1)   - index + compact + collection

Writer 协调机制：
├── 高优先级可抢占低优先级
├── 同优先级先到先得
└── 心跳超时后可接管
```

### 1.2 核心问题

**Writer 协调只解决了"谁能写"，没有解决"谁的业务被执行"**

| 场景 | 问题 |
|------|------|
| VlaudeKit 抢到 Writer | 只写 approve，不触发 index/compact |
| MemexKit 抢到 Writer | memex-rs 变成 Reader，HTTP 写入请求失败 |
| 4 组件乱序启停 | 行为不可预测 |

### 1.3 约束条件

- Kit 保留 AICliEvent 实时性（ClaudeKit Hooks）
- 业务逻辑不耦合（Memex 的归 Memex，Vlaude 的归 Vlaude）
- Collection 只能一个进程触发和写入
- 4 组件可任意启停组合

---

## 2. 解决方案：Agent 模式

### 2.1 核心思想

```
之前: Writer = 写入权 + 业务执行权（但业务执行权缺失）
之后: Agent = 唯一写入者 + Collection 触发者 + 事件推送者
      组件 = 业务计算者 + 结果提交者 + 事件订阅者
```

### 2.2 Agent 职责

```
┌─────────────────────────────────────────────────────────┐
│                         Agent                           │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  1. 唯一 Writer                                         │
│     └── 所有 DB 写入都通过 Agent                        │
│                                                         │
│  2. 文件监听 + Collection                               │
│     └── FSEvents 监听所有 AI CLI 数据源                 │
│     └── 支持多 Adapter（Claude/Codex/OpenCode/Gemini）  │
│     └── Kit 通知可增强实时性                            │
│                                                         │
│  3. 事件推送                                            │
│     └── 维护订阅列表                                    │
│     └── 新消息 → 推送给所有订阅者                       │
│                                                         │
│  4. 接收业务写入                                        │
│     └── index 结果（from memex-rs）                     │
│     └── approve 结果（from vlaude/VlaudeKit）           │
│                                                         │
│  5. 不做业务逻辑                                        │
│     └── index/compact 在 memex-rs                       │
│     └── approve 逻辑在 vlaude                           │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 2.3 组件关系

```
┌─────────────────────────────────────────────────────────┐
│                                                         │
│   Kit 层                    Daemon 层                   │
│   ┌──────────┐              ┌──────────┐               │
│   │MemexKit  │──HTTP──────→ │ memex-rs │               │
│   │ (订阅)   │              │ (订阅)   │               │
│   └────┬─────┘              └────┬─────┘               │
│        │                         │                      │
│   ┌────┴─────┐              ┌────┴─────┐               │
│   │VlaudeKit │              │ vlaude   │               │
│   │ (订阅)   │              │ (订阅)   │               │
│   └────┬─────┘              └────┬─────┘               │
│        │                         │                      │
│        │ IPC                     │ IPC                  │
│        │                         │                      │
│        └──────────┬──────────────┘                      │
│                   ▼                                     │
│           ┌──────────────┐                              │
│           │    Agent     │                              │
│           │ (唯一Writer) │                              │
│           │ (事件推送)   │                              │
│           └──────────────┘                              │
│                                                         │
│   MemexKit 仍调用 memex-rs：                            │
│   ├── website 在 ETerm 内访问                           │
│   ├── 部分数据展示需求                                  │
│   └── index/compact 逻辑在 memex-rs                     │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

---

## 3. 事件订阅机制

### 3.1 所有组件都可订阅

```
┌─────────────────────────────────────────────────────────┐
│  Agent 推送 NewMessage 事件                             │
├─────────────────────────────────────────────────────────┤
│                                                         │
│  订阅者：                                               │
│  ├── VlaudeKit      → 推送到 iOS                        │
│  ├── MemexKit       → 通知 memex-rs 做 index/compact    │
│  ├── vlaude-daemon  → 推送到 iOS（独立运行时）          │
│  └── memex-rs       → 做 index/compact（独立运行时）    │
│                                                         │
│  Kit 和 Daemon 可同时运行并订阅（不冲突）               │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

### 3.2 订阅流程

```rust
// 组件连接后订阅
Request::Subscribe {
    events: vec![EventType::NewMessage, EventType::SessionStart],
}

// Agent 维护订阅列表
struct Subscriptions {
    map: HashMap<ConnId, HashSet<EventType>>,
}

// 有新消息时广播
fn broadcast(&self, event: Event) {
    for (conn_id, subscribed) in &self.subscriptions.map {
        if subscribed.contains(&event.event_type()) {
            self.send(conn_id, event.clone());
        }
    }
}
```

### 3.3 事件类型

```rust
enum EventType {
    NewMessage,       // 新消息（Collection 完成后）
    SessionStart,     // 会话开始
    SessionEnd,       // 会话结束
}

enum Event {
    NewMessages {
        session_id: String,
        path: PathBuf,
        count: usize,
    },
    SessionStart {
        session_id: String,
        project_path: String,
    },
    SessionEnd {
        session_id: String,
    },
}
```

---

## 4. Agent 生命周期

### 4.1 二进制分发

```
位置：~/.vimo/bin/vimo-agent

部署方式：Client 运行时自动部署
├── Client 连接时检查 ~/.vimo/bin/vimo-agent 是否存在
├── 不存在 → 从源路径自动复制
└── 源路径查找顺序：
    1. ClientConfig.agent_binary_override（配置覆盖）
    2. VIMO_AGENT_PATH 环境变量
    3. Cargo target 目录（开发阶段）
    4. App bundle: ETerm.app/Contents/MacOS/vimo-agent（生产环境）

版本策略：
├── 只升级不降级
├── 部署时直接覆盖（暂不做版本比较）
└── Agent 版本号：0.0.1-beta.1（跟随 ai-cli-session-db crate）
```

### 4.2 唤起机制

```
~/.vimo/
├── bin/vimo-agent      ← 二进制（权限 0755）
├── agent.sock          ← Unix Socket（权限 0600，umask 0077）
├── agent.pid           ← PID 文件（权限 0600）
└── db/

组件启动流程：
1. connect(agent.sock)
   ├── 成功 → 版本检查 → 使用
   └── 失败 → 重试 3 次（间隔 500ms）
              └── 仍失败 → 进入步骤 2

2. 检查残留状态
   ├── PID 文件存在 + 进程活着 + 再次 connect 失败 → Agent 卡死，清理
   └── 否则 → 正常启动

3. bind(agent.sock)  ← 原子操作，保证唯一性
   ├── 成功 → 我负责启动 Agent
   └── 失败(EADDRINUSE) → 其他组件在启动，等待重试

4. 启动 Agent，等待 ready，connect

安全说明：
├── socket 文件权限 0600，仅 owner 可读写
├── 创建时设置 umask 0077，防止权限泄漏
└── 本地 Unix socket 由文件系统权限保护，无需应用层认证
```

### 4.3 退出机制

```
连接计数：
├── 组件连接 → 计数 +1
├── 组件断开 → 计数 -1
└── 计数 = 0 + 空闲超时(30s) → Agent 退出

关键实现：
├── idle_checker 持续运行，每 5 秒检测一次
├── 有连接时：重置空闲计数，取消 shutdown 标志
├── 无连接时：累计空闲时间，达到阈值设置 shutdown
└── 主循环退出条件：shutdown == true && !has_connections()
    （确保新连接进来时能取消退出）

下次有组件需要时，重新启动
```

### 4.4 容错机制

```
Agent 卡死检测：
├── 组件发送请求，设置超时(5s)
├── 超时 → 判定 Agent 卡死
├── 读取 PID 文件，kill -9
├── 删除 agent.sock 和 agent.pid
└── 重新启动 Agent

组件崩溃：
├── OS 自动关闭 socket
├── Agent 检测到连接断开
└── 正常处理，无需特殊逻辑
```

---

## 5. 版本兼容

### 5.1 策略：简化版本管理

```
版本号：仅用 Agent 版本（跟随 ai-cli-session-db crate 版本）
格式：semver（如 1.0.0）

握手时：
├── Agent 返回自己的版本号
├── 组件记录日志用于诊断
└── 暂不做版本校验（同一产品线内版本一致性由打包保证）

未来扩展：
├── 如需版本校验，在 Handshake 中加入 min_agent_version
└── Agent 版本 < min_agent_version → 返回错误
```

### 5.2 运行时不强制升级

```
场景：Agent v1.0 正在运行，新组件需要 v2.0

处理：
├── 新组件启动失败
├── 显示错误：请关闭所有程序，等待 Agent 退出后重试
└── 不强制杀掉正在运行的 Agent

原因：避免影响其他正在工作的组件
```

### 5.3 安装时升级

```
场景：用户安装新版 MEMEX（带 Agent v2.0）

处理：
├── 检查 ~/.vimo/bin/vimo-agent 版本
├── 发现 v1.0 < v2.0
├── 覆盖为 v2.0
└── 下次 Agent 启动时就是新版本

注意：如果 Agent 正在运行，不会被影响
      需要等 Agent 空闲退出后才会用新版本
```

---

## 6. IPC 协议

### 6.1 通信方式

```
Unix Socket: ~/.vimo/agent.sock
消息格式：JSONL（每条消息一行 JSON + '\n'）

Framing 规则：
├── 每条消息以 '\n' 结尾
├── 消息内部不允许换行（JSON 单行化）
├── 读取时按行解析
└── 简单、易调试、无需长度前缀

示例：
→ {"type":"Handshake","component":"memex-rs","version":"1.0.0"}\n
← {"type":"HandshakeOk","agent_version":"1.0.0"}\n
```

### 6.2 请求类型

```rust
enum Request {
    // 握手（简化为单一版本号）
    Handshake {
        component: String,      // "memex-rs" / "vlaudekit" / ...
        version: String,        // 组件版本，用于日志和诊断
    },

    // Kit 通知（增强实时性）
    NotifyFileChange {
        path: PathBuf,          // transcriptPath from ClaudeKit Hooks
    },

    // 订阅事件
    Subscribe {
        events: Vec<EventType>,
    },

    // 业务写入
    WriteIndexResult {
        session_id: String,
        data: IndexData,
    },
    WriteApproveResult {
        tool_call_id: String,
        status: ApprovalStatus,
        resolved_at: i64,
    },

    // 心跳（可选）
    Heartbeat,

    // 查询（如果需要）
    Query { ... },
}

enum Response {
    Ok,
    Error { code: i32, message: String },
    HandshakeOk {
        agent_version: String,  // Agent 版本
    },
    QueryResult { ... },
}

// 推送事件（Agent → 订阅者）
enum Push {
    NewMessages {
        session_id: String,
        path: String,
        count: usize,
        message_ids: Vec<i64>,
    },
    SessionStart {
        session_id: String,
        project_path: String,
    },
}
```

---

## 7. 文件监听

### 7.1 多 Adapter 支持

```
Agent 启动时：
1. 调用 all_watch_configs() 获取所有监听配置
2. 为每个路径创建 FSEvents 监听

支持的数据源（通过 Adapter）：
├── Claude Code    ~/.claude/projects/**/*.jsonl
├── Codex CLI      history.jsonl + rollout
├── OpenCode       ~/.local/share/opencode/
├── Gemini         session JSON
└── 未来更多...

文件变化时：
1. adapter_for_path(path) 找到对应 Adapter
2. Adapter 解析文件
3. 写入 DB
4. 推送事件给订阅者
```

### 7.2 Kit 通知增强

```
Kit 通过 ClaudeKit Hooks 获得 transcriptPath
Kit 通知 Agent: NotifyFileChange { path }
Agent 立即触发该文件的 Collection（不等 FSEvents）

作用：毫秒级实时性
兜底：FSEvents 仍在监听，不会漏消息
```

---

## 8. 数据流

### 8.1 Collection 流程

```
文件变化（FSEvents / Kit 通知）
        │
        ▼
   找到对应 Adapter
        │
        ▼
   解析文件（增量）
        │
        ▼
   写入 DB（messages 表）
        │
        ▼
   推送 NewMessage 事件给订阅者
```

### 8.2 场景 1：ETerm + MemexKit + memex-rs

```
Agent ──NewMessage──→ MemexKit ──HTTP──→ memex-rs
                                             │
                                        index/compact
                                             │
                                             ▼
                             Agent ←──WriteIndexResult
```

### 8.3 场景 2：纯 memex-rs（无 ETerm）

```
Agent ──NewMessage──→ memex-rs
                          │
                     index/compact
                          │
                          ▼
          Agent ←──WriteIndexResult
```

### 8.4 场景 3：ETerm + VlaudeKit + vlaude-daemon

```
Agent ──NewMessage──→ VlaudeKit ──Socket.IO──→ iOS
              │
              └────→ vlaude-daemon（可同时订阅）
```

### 8.5 Approve 流程

```
VlaudeKit 收到权限请求（ClaudeKit Hooks）
        │
        ▼
   处理审批逻辑
        │
        ▼
   调用 Agent: WriteApproveResult
        │
        ▼
   Agent 写入 DB
```

---

## 9. 代码结构

### 9.1 Agent 位置

```
ai-cli-session-db/
├── Cargo.toml           # 新增 [[bin]] target
├── src/
│   ├── lib.rs           # 现有库代码
│   ├── agent/           # Agent 实现（新增）
│   │   ├── mod.rs
│   │   ├── server.rs    # IPC Server (Unix Socket)
│   │   ├── watcher.rs   # FSEvents 监听（从 memex-rs 移植）
│   │   ├── handler.rs   # 请求处理
│   │   └── broadcast.rs # 事件推送
│   └── client/          # Agent Client（新增，供组件使用）
│       ├── mod.rs
│       └── connect.rs   # connect_or_start_agent()
└── bin/
    └── vimo-agent.rs    # Agent 入口（新增）
```

### 9.2 Feature Flags

```toml
[features]
default = ["reader"]
reader = []
writer = []
search = []
coordination = []  # 废弃，改用 Agent
ffi = []
agent = ["writer", "search"]  # Agent 需要写入和搜索能力
client = []  # Agent Client，供组件使用
```

### 9.3 可复用的现有代码

| 模块 | 位置 | 复用方式 |
|------|------|---------|
| Collector | `ai-cli-session-db/src/collector.rs` | 直接复用 |
| Adapter 机制 | `ai-cli-session-collector/src/adapter/` | 直接复用 |
| FileWatcher | `memex-rs/src/watcher.rs` | 移植到 Agent |
| DB 操作 | `ai-cli-session-db/src/db.rs` | 直接复用 |

---

## 10. 迁移计划

### 10.1 策略：一步到位

不做向后兼容，直接切换到新架构。

### 10.2 步骤

```
1. ai-cli-session-db 改造 ✅
   ├── 新增 agent/ 模块
   ├── 新增 client/ 模块
   ├── 新增 vimo-agent binary
   └── 编译产出 vimo-agent 二进制

2. 组件改造
   ├── memex-rs ✅ 使用 AgentClient，订阅事件，移除 FileWatcher
   ├── vlaude-core (daemon-logic) ✅ 使用 AgentClient，编译通过
   ├── VlaudeKit ✅ Swift 层 AgentClientBridge + SharedDbBridge 只读
   └── MemexKit ✅ Swift 层 AgentClientBridge + SharedDbBridge 只读

3. 移除旧的 Writer 协调机制 ✅
   ├── SharedDbBridge ✅ 已移除 register_writer/release_writer/collect
   ├── Rust 层 ✅ 无 coordination feature flag（从未实现）
   └── writer_registry 表 ✅ 已清理

4. 构建与部署 ✅
   ├── build.sh 添加 agent 构建目标
   └── Client 运行时自动部署到 ~/.vimo/bin/
```

---

## 11. 确认事项

- [x] 架构方向：Agent 模式
- [x] 业务分离：Agent 不做业务逻辑
- [x] 二进制分发：各产品都带，只升级不降级
- [x] 唤起机制：Socket bind 互斥
- [x] 退出机制：连接计数 + 空闲超时
- [x] 容错：PID 文件 + 请求超时检测
- [x] 版本兼容：不向后兼容，运行时不强制升级
- [x] 事件订阅：所有组件都可订阅（MemexKit 需要订阅才能触发 index/compact）
- [x] 代码结构：Agent 和 Client 在 ai-cli-session-db 中

### Codex Review 改进（2026-01-23）

- [x] 消息 framing：JSONL 格式（每条消息一行 JSON + '\n'）
- [x] 启动检测：connect 失败后重试 3 次（500ms 间隔），避免误杀正在启动的 Agent
- [x] 版本简化：去掉 protocol_version，仅用 agent_version
- [x] Socket 权限：明确 0600 权限 + umask 0077

---

## 12. 实现记录

### Phase 1: Agent/Client 框架（2026-01-23）

已完成：
- `ai-cli-session-db/src/agent/` - Agent 服务端实现
- `ai-cli-session-db/src/client/` - Client 连接逻辑
- `ai-cli-session-db/src/bin/vimo_agent.rs` - Agent 入口
- `ai-cli-session-db/src/protocol.rs` - IPC 协议定义

### Phase 2: 组件改造（2026-01-23）

已完成：
- `memex/memex-rs/src/agent_client.rs` - memex AgentClient 封装
- `vlaude/packages/vlaude-core/daemon-logic/src/agent_client.rs` - vlaude AgentClient

### Bug 修复（2026-01-23）

**Bug 1: Agent 空闲退出逻辑错误**
- 问题：shutdown 信号发出后，新连接进来也无法阻止退出
- 修复：`server.rs` 主循环退出条件改为 `shutdown && !has_connections()`
- 修复：`idle_checker` 有连接时重置 shutdown 标志

**Bug 2: Agent 二进制部署**
- 问题：Client 找不到 ~/.vimo/bin/vimo-agent
- 修复：`connect.rs` 添加 `find_agent_binary()` 和 `deploy_agent()`
- 实现：运行时自动从 target/ 或 app bundle 部署到 ~/.vimo/bin/

### Phase 3: Swift 层改造 + 构建脚本（2026-01-24）

已完成：
- `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/AgentClientBridge.swift` - VlaudeKit Agent Client
- `ETerm/Plugins/MemexKit/Sources/MemexKit/AgentClientBridge.swift` - MemexKit Agent Client
- `scripts/build.sh` - 添加 `agent` 构建目标

### Phase 4: Swift 层只读改造（2026-01-24）

**MemexKit 修改：**
- `SharedDbBridge.swift` - 移除 Writer 协调、写入、采集方法（889 → 472 行）
- `MemexService.swift` - 移除 `registerAndCollect()`，`collectByPath()` 改用 AgentClient

**VlaudeKit 修改：**
- `SharedDbBridge.swift` - 移除 Writer 协调、写入、采集方法（801 → 414 行）
  - 保留 `updateApprovalStatusByToolCallId()`（AgentClient FFI 暂未实现）
- `VlaudeClient.swift` - 移除 `initSharedDb()` 中的 `registerAndCollect()`，删除 `indexSession()`
- `VlaudePlugin.swift` - 移除 `register()` 调用，`indexSession` 改用 `agentClient.notifyFileChange()`

**架构状态：**
- Swift 层 SharedDbBridge 现在是纯只读（查询 + approval 更新）
- 所有数据采集和写入通过 AgentClient → vimo-agent
- 风险已消除：不再存在双写冲突

**Phase 4.1: AgentClient writeApproveResult 实现（2026-01-24）**

Rust 层：
- `protocol.rs` - ApprovalStatus 添加 Pending 状态
- `client/connect.rs` - AgentClient 添加 `write_approve_result` 方法
- `client/ffi.rs` - 导出 `agent_client_write_approve_result` FFI 函数
- `agent/handler.rs` - 处理 Pending 状态转换

Swift 层：
- `AgentClientBridge.swift` - 添加 `AgentApprovalStatusSwift` 枚举和 `writeApproveResult` 方法
- `VlaudePlugin.swift` - 两处调用改为使用 AgentClient
- `SharedDbBridge.swift` - 移除 ApprovalStatus 枚举和 updateApprovalStatusByToolCallId 方法（414 → 353 行）

**Phase 5: 清理验证（2026-01-25）**

验证结果：
- `coordination` feature flag - 从未作为 feature 存在，仅文档描述
- `writer_registry` 表代码 - 已在早期清理
- `session_db_update_approval_status_by_tool_call_id` FFI - 已移除
- Cargo.toml description - 已更新为 Agent 架构描述

✅ **Agent 架构迁移完成**
