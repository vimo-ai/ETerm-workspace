# Agent V2: 读写分离架构

> 创建时间: 2026-02-06
> 状态: ✅ Phase 1-4 已实现，Phase 5 待实施（file watch 保底 + 剪枝优化）
> 前置文档: [session-db-agent.md](./session-db-agent.md)（V1，本文档替代）

## 1. 问题

V1 架构中 vimo-agent 同时承担写入和事件广播：

```
vimo-agent
├── 写入: file watch → collect JSONL → DB write
├── 广播: Unix Socket → Broadcaster → Push channel → 多个 subscriber
└── 写代理: 接受客户端写请求 → DB write
```

**广播链路的致命缺陷**：

1. Broadcaster channel 满 → `blocking_send` 阻塞 debouncer 回调线程 → file watcher 死亡
2. 一旦 watcher 死，所有下游全部断流（VlaudeKit 推送停、MemexKit compact 停）
3. 重连机制无法恢复已死的 watcher，只能重启 agent

根本原因：**推送模型（Push）在多消费者场景下不可靠**。任一消费者慢都会反压到源头。

## 2. 设计原则

- **agent 只写不推**：砍掉 Broadcaster、Subscribe、Push channel
- **各端自行触发读**：Kit 走 AICliKit event，独立进程走 file watch
- **读写物理分离**：写走 agent RPC，读走 DB/JSONL 直接访问
- **零耦合**：任一链路故障不影响其他链路

### 2.1 三大核心保证

整个架构围绕三个不可妥协的维度设计：

| 维度 | 要求 | 保证机制 |
|------|------|----------|
| **实时性** | 事件触发到数据可见 < 1s | AICliKit event 进程内零延迟；file watch 2s debounce 保底 |
| **幂等性** | 重复触发不产生副作用 | fileOffset 主游标 + UUID 去重窗口，所有读写操作幂等 |
| **数据完整** | 零丢失，任何故障后可恢复 | JSONL 是唯一真相源（append-only）；游标持久化；冷启动全量补偿 |

## 3. 新架构

```
                    Claude Code writes JSONL
                              │
              ┌───────────────┼───────────────┬─────────────────┐
              │               │               │                 │
              ▼               ▼               ▼                 ▼
          vimo-agent      VlaudeKit        MemexKit        memex-rs / daemon
         (file watch)   (AICliKit event)  (AICliKit event)   (file watch)
              │               │               │                 │
              ▼               ▼               ▼                 ▼
           collect        read JSONL      notify agent      detect change
           → DB写入       → push iOS      → agent collect    → query DB
                                          → trigger compact   → compact/index
```

### 3.1 vimo-agent（纯写代理）

**职责**：DB 的唯一写入者，保证写操作串行化。

| 功能 | 触发方式 | 说明 |
|------|----------|------|
| JSONL → DB | 自身 file watch | 文件监听 → parse → insert messages/sessions/projects |
| NotifyFileChange | RPC 请求 | 客户端通知立即采集指定文件（**同步语义：RPC 返回即 collect 完成**） |
| WriteApproveResult | RPC 请求 | VlaudeKit 审批结果写入 |
| WriteCompactResult | RPC 请求 | memex-rs 摘要结果写入 |
| WriteIndexResult | RPC 请求 | memex-rs 向量索引标记写入 |

**砍掉的功能**：
- ~~Broadcaster 模块~~
- ~~Subscribe / Unsubscribe 请求~~
- ~~Push channel 管理~~
- ~~事件分发（NewMessage / SessionStart / SessionEnd）~~

**RPC 通道**：保留 Unix Socket，仅做 request-response。客户端发起、agent 响应，无背压风险。

### 3.2 VlaudeKit（iOS 消息推送）

**主触发**：AICliKit event（进程内，零延迟）
**保底**：file watch（AICliKit 未触发时兜底）

```
AICliKit event                       file watch (保底)
     │                                    │
     ▼                                    ▼
 aicli.responseComplete             JSONL 文件变化
 aicli.promptSubmit                       │
 aicli.permissionRequest                  │
     │                                    │
     └──────────┬─────────────────────────┘
                │
                ▼
    SessionReader.readMessages(JSONL, fromOffset)
    → 只消费完整行（不完整的末尾行丢弃，下次重读）
    → UUID 去重窗口过滤
    → processNewMessages
    → push to iOS
    → 更新游标（offset 前进到最后一条完整行末尾）
```

**读取路径**：直接读 JSONL（SessionReader），不经 DB，不依赖 agent collect 时序。

**写请求**：writeApproveResult → agent RPC

**砍掉的依赖**：
- ~~AgentClientBridge 事件订阅~~（Subscribe / Push 回调）
- ~~AgentClientDelegate~~
- ~~initializeAgentClient / attemptAgentReconnect~~

**保留的 agent 交互**：仅 RPC 写请求（request-response）

### 3.3 MemexKit（搜索 + 摘要触发）

**触发**：AICliKit event（`aicli.responseComplete`）

```
aicli.responseComplete
     │
     ├──→ agent RPC: notifyFileChange(path)  → agent collect → DB 就绪（同步）
     │
     └──→ HTTP: memex-rs triggerCompact(sessionId)  → memex-rs 读 DB → compact
```

**读取路径**：SharedDbBridge 读 DB（搜索、统计等 UI 功能）

**砍掉的依赖**：
- ~~AgentClientBridge 事件订阅~~
- ~~AgentClientDelegate (handleAgentNewMessages)~~

**保留的 agent 交互**：notifyFileChange RPC（触发采集）

### 3.4 memex-rs（独立摘要 + 向量索引）

**触发**：file watch（监听 JSONL 文件变化）

```
JSONL 文件变化
     │
     ▼
  detect change
     │
     ├──→ agent RPC: notifyFileChange(path)   ← 确保 agent 先 collect（同步）
     │
     ├──→ RPC 返回 = collect 完成，DB 已包含最新数据
     │
     ├──→ compact → writeCompactResult → agent RPC
     └──→ vector index → writeIndexResult → agent RPC
```

**竞态处理**：memex-rs 触发时 agent 可能尚未 collect，DB 里没有新数据。处理策略：

1. 先发 `notifyFileChange` RPC（同步语义，返回即 collect 完成）
2. 再 query DB 获取新消息 → compact/index
3. 若 RPC 失败，指数退避重试

**按需触发优化**：不是每次 file watch 都发 RPC。先检查 DB 中该 session 的 `updated_at` 或 `message_count` 是否已反映最新变化（与 JSONL mtime/size 对比），已最新则跳过 RPC，避免高频场景放大 agent 负载。

**读取路径**：DbReader 读 DB

**写请求**：writeCompactResult / writeIndexResult → agent RPC

**砍掉的依赖**：
- ~~AgentClient 事件订阅（NewMessage push）~~

### 3.5 vlaude-daemon（独立 iOS 推送，无 ETerm 时）

**触发**：file watch（监听 JSONL 文件变化）

```
JSONL 文件变化
     │
     ▼
  session-reader 读 JSONL → push to iOS (via vlaude-server)
```

**与 VlaudeKit 互斥**：VlaudeKit 运行时 daemon 让位，不会同时运行。

## 4. 读侧游标协议

所有读侧消费者必须遵循统一的游标规范，保证幂等和数据不丢失。

### 4.1 游标定义

```
CursorState {
    sessionId: String          // 会话 ID
    fileOffset: i64            // JSONL 文件读取偏移量（主游标）
    lastMessageUUID: String?   // 最后处理的消息 UUID（去重窗口，可选）
    fileMtime: i64             // 文件修改时间（变化检测）
    fileSize: i64              // 文件大小（变化检测）
}
```

**主游标是 `fileOffset`**，不是 UUID。JSONL 是 append-only，offset 天然单调递增。
UUID 不具备排序语义，仅用作去重窗口（防止同一消息因 offset 边界被处理两次）。

### 4.2 游标规则

| 规则 | 说明 |
|------|------|
| **offset 单调递增** | `fileOffset` 只能前进，不能回退。每次读取从 offset 开始，只消费完整行 |
| **完整行保证** | 读取时遇到不完整的末尾行（无换行符结尾），丢弃并保持 offset 不变，下次重读 |
| **UUID 去重窗口** | 维护最近 N 条已处理消息的 UUID 集合，重复 UUID 跳过（防 offset 边界重叠） |
| **持久化存储** | 游标必须持久化到磁盘，进程重启后从上次 offset 恢复 |
| **原子写入** | 游标文件更新使用"写临时文件 + rename"，防止断电/崩溃导致游标损坏 |
| **冷启动补偿** | 无持久化游标时（首次启动或游标丢失），全量读取一次，建立基准 |
| **游标失效降级** | fileSize < fileOffset 时（文件被清理/截断），重置 offset=0，全量读取 |

### 4.3 各组件游标存储

| 组件 | 游标位置 | 持久化方式 |
|------|----------|------------|
| VlaudeKit | `~/.vimo/plugins/vlaude/cursors.json` | JSON 文件，sessionId → CursorState |
| MemexKit | 不需要（不直接读消息，只触发 agent collect） | — |
| memex-rs | DB 内 `incremental_state` 表 | agent collect 时自动维护 |
| vlaude-daemon | `~/.vimo/daemon/cursors.json` | JSON 文件，格式同 VlaudeKit |
| vimo-agent | DB 内 `incremental_state`（offset/mtime/size/inode） | 已实现 |

### 4.4 VlaudeKit / daemon 互斥与游标接力

VlaudeKit 和 vlaude-daemon 功能互斥，通过进程锁判定：

```
~/.vimo/locks/vlaude-push.lock    (flock 排他锁)
~/.vimo/plugins/vlaude/cursors.json  (共享游标文件)
```

| 场景 | 行为 |
|------|------|
| ETerm 启动（VlaudeKit 加载） | 获取 flock → 读取 cursors.json → 从游标位置恢复推送 |
| ETerm 退出（VlaudeKit 卸载） | 写回 cursors.json → 释放 flock |
| daemon 启动 | 尝试 flock → 成功则接管 → 读取 cursors.json 恢复 |
| daemon 检测到锁被占 | 退让，不启动推送功能 |
| 进程异常退出 | **flock 由内核自动释放**（无需手动清理锁文件），cursors.json 以最后一次写回为准 |

游标文件共享确保切换时不重复推送、不丢消息。

## 5. 写操作幂等保证

agent 作为唯一写者，所有写操作必须幂等：

| 写操作 | 幂等机制 | 说明 |
|--------|----------|------|
| insert message | UPSERT by (session_id, uuid) | 同一消息重复 collect 不会产生重复记录 |
| insert session | UPSERT by session_id | 同一会话重复 collect 安全 |
| insert project | UPSERT by path | 同一项目重复 collect 安全 |
| update approval | UPDATE by tool_call_id | 幂等，相同状态写入无副作用 |
| upsert talk_summary | UPSERT by (session_id, talk_id) | 幂等，覆盖写入 |
| mark indexed | UPDATE SET indexed=1 | 幂等，重复标记无副作用 |

**RPC 写请求失败处理**：

- 调用方超时后可安全重试（所有写操作幂等）
- agent 端 busy_timeout=5000ms，写锁等待不超过 5 秒
- 调用方建议：失败后指数退避重试，最多 3 次

## 6. 文件监听剪枝策略

独立进程（agent / memex-rs / daemon）依赖 file watch，需要剪枝控制开销：

| 策略 | 说明 |
|------|------|
| 路径过滤 | 只 watch `~/.claude/projects/` 下的 `.jsonl` 文件 |
| mtime/size/inode | DB 中 IncrementalState 记录上次状态，无变化则跳过 |
| debounce | 2 秒去抖，合并连续写入 |
| 活跃过滤 | 只 watch 近期活跃的 session 文件（如 24h 内有变动） |
| 冷启动扫描 | 进程首次启动时全量扫描一次，捕获停机期间的变化 |
| offset 增量读 | 从上次读到的 offset 开始，`seek` 到位置读增量，不重新打开全文件 |
| 目录层级 | 先 watch 目录级变化，再精确到文件级 |

## 7. 实施计划

**灰度原则**：每个 Phase 先并行引入新逻辑，验证稳定后再删除旧链路。不做先删后建。

### Phase 1: VlaudeKit 去 AgentClient 事件订阅 ✅ 已实现

改动范围：`ETerm/Plugins/VlaudeKit/`

1. ✅ **新增**：在 `handleClaudeResponseComplete` / `handleClaudePromptSubmit` / `handleClaudePermissionPrompt` 中加入消息读取 + 推送逻辑
2. ✅ **新增**：游标持久化（`cursors.json`），替换内存中的 `lastMessageUUIDs`
3. ✅ **新增**：进程锁机制（`vlaude-push.lock`）
4. ⏳ **验证**：新旧链路并行跑，对比推送结果一致性
5. ⏳ **删除**（验证通过后）：`AgentClientDelegate` 事件处理、initializeAgentClient、attemptAgentReconnect
6. ✅ **保留**：agent RPC 写请求（AgentClientBridge 仅保留 writeApproveResult）

**实现细节**：

改动文件：

| 文件 | 改动 |
|------|------|
| `SessionReader.swift` | 新增 `readMessagesFromOffset`：纯 Swift 增量 JSONL 读取，seek 到 offset → 读增量 → 完整行保证 |
| `VlaudePlugin.swift` | 新增 `CursorState` / `CursorStore`：fileOffset 主游标 + UUID 去重窗口，原子写入 `~/.vimo/plugins/vlaude/cursors.json` |
| `VlaudePlugin.swift` | 新增 `acquirePushLock` / `releasePushLock`：flock `~/.vimo/locks/vlaude-push.lock`，activate 获取 / deactivate 释放 |
| `VlaudePlugin.swift` | 新增 `readAndPushNewMessages`：变化检测 → 游标失效检查 → 增量读 → UUID 去重 → push → 持久化 |

并行验证期：V1（AgentClient push）和 V2（AICliKit event）同时运行。V2 触发更快先推送，V1 后到可能推重复。iOS 端通过 UUID 去重。

### Phase 2: MemexKit 去 AgentClient 事件订阅 ✅ 已实现

改动范围：`ETerm/Plugins/MemexKit/`

1. ✅ 删除 `AgentClientDelegate` 实现（`MemexService.swift`）
2. ✅ 删除事件订阅（subscribe）、重连逻辑（attemptReconnect）
3. ✅ `initAgentClient` → `initAgentRPC`（仅连接不订阅）
4. ✅ 清理 `AgentClientBridge.swift`：删除 delegate/event types/subscribe/callback，仅保留 RPC
5. ✅ 保留 `notifyFileChange` RPC 调用
6. ✅ `aicli.responseComplete`（MemexPlugin.handleEvent）仍是主触发，逻辑不变

### Phase 3: agent 砍掉广播模块 ✅ 已实现

改动范围：`ai-cli-session-db/src/`

1. ✅ `broadcaster.rs` → `ConnectionManager`（仅连接管理，移除 subscribe/broadcast）
2. ✅ `handler.rs` 删除 Subscribe / Unsubscribe 处理
3. ✅ `watcher.rs` 删除 broadcaster 引用和 broadcast 调用
4. ✅ `protocol.rs` 删除 Push / Event / EventType 枚举，删除 Subscribe / Unsubscribe 请求
5. ✅ `lib.rs` 删除 Event / EventType / Push 的 re-export
6. ✅ `client/connect.rs` 删除 subscribe / recv_push / push_receiver
7. ✅ `client/ffi.rs` 删除 subscribe / push callback / disconnect callback / event loop
8. ✅ C headers (VlaudeKit + MemexKit) 删除 AgentEventType / AgentPushCallback / AgentDisconnectCallback
9. ✅ VlaudeKit Swift: 删除 V1 AgentClientDelegate / initializeAgentClient / attemptAgentReconnect
10. ✅ MemexKit Swift: 删除 AgentClientDelegate / initAgentClient → initAgentRPC

### Phase 4: memex-rs 去 AgentClient 事件订阅 ✅ 已实现

改动范围：`memex/memex-rs/src/`

1. ✅ 删除 `agent_client.rs` 整个模块
2. ✅ `lib.rs` 移除 `pub mod agent_client;`
3. ✅ `main.rs` 移除 Agent 事件循环（compact_tx + index_tx + run_agent_event_loop_with_reconnect）
4. ✅ `Cargo.toml` 移除 `client` feature 依赖

### Phase 5: file watch 保底 + 剪枝优化

1. VlaudeKit 添加 file watch 保底（AICliKit event 未覆盖的边缘场景）
2. 各端 file watch 统一基础策略 + 各组件可调参数
3. 冷启动全量扫描验证
4. 性能测试与调优

## 8. 向后兼容

- Agent RPC 接口保持不变（NotifyFileChange、Write* 请求）
- SharedDbBridge / DbReader 读接口不变
- AICliKit 事件接口不变
- 变化完全在各组件内部，对外接口无 breaking change

## 9. 风险与缓解

| 风险 | 缓解 |
|------|------|
| AICliKit event 漏触发 | file watch 保底机制 |
| agent 未及时 collect，读 DB 数据不全 | notifyFileChange 同步 RPC（返回即 collect 完成） |
| file watch 开销 | 剪枝策略（路径/mtime/debounce/活跃过滤） |
| VlaudeKit 读 JSONL 与 Claude Code 写 JSONL 竞争 | JSONL 是 append-only；只消费完整行，不完整末尾丢弃 |
| 双触发重复处理（AICliKit + file watch 同时触发） | 游标幂等：offset + UUID 去重窗口 |
| 进程重启丢失游标 | cursors.json 持久化（原子写入）+ 冷启动全量补偿 |
| JSONL 文件被清理/截断 | 游标失效降级：fileSize < offset 时重置，全量读取 |
| agent RPC 写请求失败 | 所有写操作幂等，调用方指数退避重试（最多 3 次） |
| 多进程同时 file watch 同一路径 | 各进程独立 watch，debounce 控制频率，处理逻辑幂等 |
| VlaudeKit / daemon 切换时数据不连续 | 共享 cursors.json + flock 互斥锁（内核自动释放），游标接力 |
| memex-rs 高频 file watch 放大 agent 负载 | 按需触发：先检查 DB 是否已最新，跳过不必要的 notifyFileChange |
