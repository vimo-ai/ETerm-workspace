# vimo-agent 事件中心架构设计

> 创建时间: 2025-01-27
> 最后更新: 2025-01-27
> 状态: 📝 设计中

## 1. 背景与问题

### 1.1 当前架构

```
Claude Code 会话数据
    ↓
┌─────────────────────────────────────────────────────┐
│                  触发层                              │
│                                                     │
│   FileWatcher              Hooks (claude_hook.sh)   │
│   (vimo-agent 内部)         (ETerm 专属)             │
│                                     ↓               │
│                            ETerm Unix Socket        │
│                                     ↓               │
│                               AICliKit              │
│                                     ↓               │
└────────────────────────────────────┬────────────────┘
                                     ↓
                               vimo-agent
                                     ↓
                            ┌────────┴────────┐
                            ↓                 ↓
                         memex             ETerm
```

### 1.2 核心问题

**Hooks 路径与 ETerm 强耦合，memex 独立场景无法享受即时性**

| 场景 | FileWatcher | Hooks | 实际效果 |
|------|-------------|-------|----------|
| 只有 memex | ✅ 2秒延迟 | ❌ 不触发 | 只能靠 FileWatcher |
| ETerm + memex | ✅ 2秒延迟 | ✅ 即时 | Hooks 优先 |
| 从 ETerm 切换到 memex | ✅ | ❌ claude_hook.sh exit 0 | 即时性丧失 |
| 安装了 ETerm 但未启动 | ✅ | ❌ 检测到非 ETerm 环境 | 即时性丧失 |

**根本原因**：
- `claude_hook.sh` 检测 `ETERM_TERMINAL_ID` 环境变量
- 非 ETerm 环境直接 `exit 0`，不做任何通知
- Hooks 信息只流向 ETerm，不流向共享层 vimo-agent

### 1.3 设计目标

1. **解耦**：Hooks 不依赖 ETerm 存在，memex 独立场景也能即时收集
2. **统一**：vimo-agent 作为事件中心，所有消费者平等订阅
3. **兼容**：ETerm 现有功能（Tab 装饰、权限审批）不受影响
4. **可靠**：FileWatcher 作为兜底，保证最终一致性

---

## 2. 目标架构

### 2.1 架构图

```
┌─────────────────────────────────────────────────────┐
│                  触发层                              │
│                                                     │
│   FileWatcher              Hooks (claude_hook.sh)   │
│   (文件系统监听)            (Claude Code 主动通知)    │
│   - 2秒 debounce           - 即时                   │
│   - 只有 path              - 完整事件 JSON           │
│                                                     │
└──────────────┬────────────────────┬─────────────────┘
               │                    │
               ↓                    ↓
┌─────────────────────────────────────────────────────┐
│              vimo-agent（事件中心）                   │
│                                                     │
│   ┌─────────────────────────────────────────────┐   │
│   │              事件接收                        │   │
│   │  - FileChange (内部 FileWatcher)            │   │
│   │  - HookEvent (外部请求)                     │   │
│   └─────────────────────────────────────────────┘   │
│                        ↓                            │
│   ┌─────────────────────────────────────────────┐   │
│   │              去重 & 处理                     │   │
│   │  - 基于 session_id + file_offset 去重       │   │
│   │  - Collection 执行                          │   │
│   └─────────────────────────────────────────────┘   │
│                        ↓                            │
│   ┌─────────────────────────────────────────────┐   │
│   │              事件广播                        │   │
│   │  - Push::NewMessages                        │   │
│   │  - Push::HookEvent (新增)                   │   │
│   └─────────────────────────────────────────────┘   │
│                                                     │
└──────────────┬────────────────────┬─────────────────┘
               │                    │
        ┌──────┴──────┐      ┌──────┴──────┐
        ↓             ↓      ↓             ↓
    ┌───────┐    ┌───────┐  ┌───────┐   ┌───────┐
    │ memex │    │ ETerm │  │ vlaude│   │ future│
    │       │    │       │  │       │   │       │
    │订阅:   │    │订阅:   │  │订阅:   │   │       │
    │NewMsg │    │NewMsg │  │NewMsg │   │       │
    │       │    │HookEvt│  │       │   │       │
    └───────┘    └───────┘  └───────┘   └───────┘
```

### 2.2 核心变化

| 组件 | 当前 | 目标 |
|------|------|------|
| claude_hook.sh | → ETerm Socket | → vimo-agent |
| ETerm AICliKit | 自己开 Socket Server | 订阅 vimo-agent 事件 |
| vimo-agent | 只接收 NotifyFileChange | 接收完整 HookEvent |
| memex | 只订阅 NewMessages | 不变（只关心 NewMessages） |

---

## 3. 事件分层

### 3.1 事件类型与语义

| 层级 | 事件 | 来源 | 信息量 | 消费者 | 语义 |
|------|------|------|--------|--------|------|
| L0 | FileChange | FileWatcher | `{ path }` | vimo-agent 内部 | 触发信号 |
| L1 | NewMessages | Collection 完成 | `{ session_id, path, count, message_ids }` | memex, ETerm, vlaude | **持久状态变更** |
| L2 | HookEvent | Claude Code Hooks | `{ event_type, session_id, ... }` | ETerm | **瞬时通知** |

> **事件语义明确**：
> - **NewMessages (L1)**：表示数据库中有新消息，是"持久状态指示"，订阅者应以此为准
> - **HookEvent (L2)**：表示 Claude Code 发生了某个事件，是"瞬时通知"，用于 UI 即时反馈
>
> ETerm 同时订阅两者：HookEvent 用于 Tab 装饰等 UI 功能，NewMessages 用于数据同步

### 3.2 HookEvent 结构

```rust
/// Claude Code Hook 事件（L2）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEvent {
    /// 事件 ID（用于调试追踪，可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// 时间戳（毫秒，可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
    /// 事件类型（使用 string 保证向前兼容）
    pub event_type: String,
    /// 会话 ID
    pub session_id: String,
    /// transcript 文件路径
    pub transcript_path: Option<String>,
    /// 工作目录
    pub cwd: Option<String>,
    /// 用户输入（UserPromptSubmit）
    pub prompt: Option<String>,
    /// 工具名称（PermissionRequest）
    pub tool_name: Option<String>,
    /// 工具输入（PermissionRequest）
    pub tool_input: Option<serde_json::Value>,
    /// 工具调用 ID（PermissionRequest）
    pub tool_use_id: Option<String>,
    /// 通知类型（Notification）
    pub notification_type: Option<String>,
    /// 通知消息（Notification）
    pub message: Option<String>,
}

/// 已知的 Hook 事件类型（开放枚举，使用 string 匹配）
/// 未知类型应静默忽略，保证向前兼容
pub mod HookEventType {
    pub const SESSION_START: &str = "SessionStart";
    pub const SESSION_END: &str = "SessionEnd";
    pub const USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
    pub const STOP: &str = "Stop";
    pub const NOTIFICATION: &str = "Notification";
    pub const PERMISSION_REQUEST: &str = "PermissionRequest";
    // 未来 Claude Code 可能新增其他类型，消费者应忽略未知类型
}
```

---

## 4. 去重机制

### 4.1 问题场景

```
t0: Claude Code 写入 message
t1: Hooks 触发 → vimo-agent 收到 HookEvent → Collection
t2: FileWatcher 检测到变化 → vimo-agent 内部触发 → Collection (重复!)
```

### 4.2 去重策略

**基于 message_id 去重**（而非 file_offset）：

> **设计决策**：file_offset 在日志旋转、truncate、并发写入场景下不可靠。
> 使用 message_id 更稳定，因为 Collection 过程已经会记录已处理的消息。

```rust
impl Agent {
    fn collect(&self, session_id: &str, file_path: &Path) -> CollectionResult {
        // Collector 内部已有去重逻辑：
        // 1. 读取文件中的所有消息
        // 2. 对比数据库中已存在的 message_id
        // 3. 只插入新消息
        // 4. 返回 new_message_ids

        let collector = Collector::new(&self.db);
        collector.collect_by_path(file_path)
        // CollectionResult { messages_inserted, new_message_ids }
    }
}
```

**关键点**：
- 去重发生在 **Collection 内部**，不在触发层
- Hooks 和 FileWatcher 都可以触发 Collection
- Collection 是**幂等的**：重复调用不会产生重复数据
- 性能优化：通过 `last_message_id` 快速跳过已处理部分

### 4.3 触发优先级

| 触发源 | 延迟 | 优先级 | 行为 |
|--------|------|--------|------|
| HookEvent | 即时 | 高 | 立即 Collection |
| FileWatcher | 2秒 | 低 | Collection（内部去重） |

**注意**：两者触发的 Collection 都是幂等的，不需要在触发层去重。

---

## 5. 协议扩展

### 5.1 协议版本

```rust
/// 协议版本（在 Handshake 中携带）
pub const PROTOCOL_VERSION: u32 = 2;  // 新增 HookEvent 支持

/// 握手请求
pub struct Handshake {
    pub component: String,
    pub version: String,
    pub protocol_version: Option<u32>,  // 新增，向后兼容
}

/// 握手响应
pub struct HandshakeOk {
    pub agent_version: String,
    pub protocol_version: u32,  // 新增
}
```

> **向后兼容策略**：
> - 新字段使用 `Option`，旧客户端不传也能工作
> - 未知的 Request/Push 类型应静默忽略
> - 版本号仅用于能力探测，不用于严格校验

### 5.2 新增 Request

```rust
/// 请求类型
pub enum Request {
    // ... 现有 ...

    /// Hook 事件（来自 claude_hook.sh）
    HookEvent(HookEvent),
}
```

### 5.3 新增 Push

```rust
/// 推送事件
pub enum Push {
    // ... 现有 ...

    /// Hook 事件广播
    HookEvent(HookEvent),
}
```

### 5.4 新增 EventType

```rust
pub enum EventType {
    NewMessage,
    SessionStart,
    SessionEnd,
    HookEvent,  // 新增
}
```

---

## 6. claude_hook.sh 改造

### 6.1 当前逻辑

```bash
# 检查 ETerm 环境
if [ -z "$ETERM_TERMINAL_ID" ]; then
    exit 0  # 非 ETerm 环境，直接退出
fi

# 发送到 ETerm Socket
echo "$json" | nc -U "$ETERM_SOCKET_DIR/claude.sock"
```

### 6.2 目标逻辑

```bash
# 1. 总是通知 vimo-agent（核心路径）
AGENT_SOCKET="${VIMO_DATA_DIR:-$HOME/.vimo/db}/agent.sock"
if [ -S "$AGENT_SOCKET" ]; then
    # 发送 HookEvent 请求
    echo '{"type":"HookEvent",...}' | nc -U "$AGENT_SOCKET"
fi

# 2. 如果在 ETerm 环境，额外通知 ETerm Socket（渐进迁移期间保留）
if [ -n "$ETERM_TERMINAL_ID" ] && [ -n "$ETERM_SOCKET_DIR" ]; then
    echo "$json" | nc -U "$ETERM_SOCKET_DIR/claude.sock"
fi
```

### 6.3 迁移策略

| 阶段 | claude_hook.sh 行为 | ETerm 行为 |
|------|---------------------|------------|
| Phase 1 | 双写（vimo-agent + ETerm Socket） | 保持现有 Socket Server |
| Phase 2 | 双写 | 改为订阅 vimo-agent，Socket Server 降级为 fallback |
| Phase 3 | 只写 vimo-agent | 移除 Socket Server |

---

## 7. ETerm 改造

### 7.1 当前架构

```
ClaudeSocketServer (监听 claude.sock)
    ↓
ClaudeProvider (解析事件)
    ↓
AICliKitPlugin (处理 UI 逻辑)
```

### 7.2 目标架构

```
vimo-agent (订阅 HookEvent)
    ↓
HookEventReceiver (新增)
    ↓
AICliKitPlugin (处理 UI 逻辑，接口不变)
```

### 7.3 兼容层

```swift
/// 从 vimo-agent 接收 HookEvent，转换为 AICliEvent
class HookEventReceiver {
    func onHookEvent(_ event: HookEvent) {
        let aiCliEvent = mapToAICliEvent(event)
        delegate?.onEvent(aiCliEvent)
    }

    private func mapToAICliEvent(_ hook: HookEvent) -> AICliEvent {
        // HookEvent → AICliEvent 映射
        // 保持 AICliKitPlugin 接口不变
    }
}
```

---

## 8. 边界情况

### 8.1 vimo-agent 未运行

```
Hooks 触发 → nc 失败 → 静默跳过
FileWatcher → 不工作
启动后 → FileWatcher 触发全量扫描补救
```

**处理**：claude_hook.sh 中 `nc` 失败应静默（`|| true`），不阻塞 Claude Code。

### 8.2 Hooks 未配置

```
用户未配置 hooks → 只有 FileWatcher 工作 → 2秒延迟
```

**处理**：这是预期行为，FileWatcher 是兜底。

### 8.3 ETerm 和 memex 都未运行

```
vimo-agent 运行 → Hooks/FileWatcher 触发 Collection → 写入数据库
事件广播 → 无订阅者 → 静默丢弃
```

**处理**：数据已持久化，订阅者启动后可查询。

### 8.4 网络/Socket 瞬断

```
nc 发送失败 → 静默跳过
FileWatcher 2秒后触发 → 补救
```

**处理**：双重保障，最终一致。

---

## 9. 实施计划

### Phase 1: 协议扩展（vimo-agent）

- [ ] 新增 `Request::HookEvent`
- [ ] 新增 `Push::HookEvent`
- [ ] 新增 `EventType::HookEvent`
- [ ] Handler 处理 HookEvent → 触发 Collection → 广播

### Phase 2: claude_hook.sh 改造

- [ ] 增加 vimo-agent 通知路径
- [ ] 保留 ETerm Socket 通知（双写）
- [ ] 测试非 ETerm 环境

### Phase 3: 去重机制

- [ ] 实现 file_offset 跟踪
- [ ] FileWatcher 触发时检查是否已处理
- [ ] 测试 Hooks + FileWatcher 并发场景

### Phase 4: ETerm 迁移（可选）

- [ ] 新增 HookEventReceiver
- [ ] AICliKit 改为订阅 vimo-agent
- [ ] 移除 ClaudeSocketServer

---

## 10. 风险与缓解

| 风险 | 缓解措施 |
|------|----------|
| vimo-agent 单点故障 | FileWatcher 兜底 + 启动后补救 |
| 迁移期间功能回退 | 双写策略，渐进迁移 |
| 协议不兼容 | 版本号 + 向后兼容设计 |
| 性能影响 | localhost 通信，延迟可忽略 |

---

## 11. 设计决策

### 11.1 采纳的建议

| 建议 | 采纳理由 |
|------|----------|
| 去重用 message_id 而非 file_offset | file_offset 在日志旋转、truncate 场景不可靠 |
| 事件语义明确（HookEvent vs NewMessages） | 架构清晰的基本要求 |
| 协议版本号 | 低成本，未来兼容 |
| 开放枚举（event_type 用 string） | 防止新事件类型破坏解析 |
| event_id + timestamp | 调试追踪有帮助（设为可选） |

### 11.2 暂不采纳的建议

| 建议 | 不采纳理由 |
|------|------------|
| 失败队列/重试策略 | 本地 Unix Socket 几乎不会失败；FileWatcher 已是 fallback |
| 可观测性埋点/指标 | 当前阶段不需要，出问题看日志即可 |
| 敏感数据脱敏 | 单用户本地数据，不存在多租户隔离问题 |
| 本地持久化队列 | FileWatcher 2秒兜底已足够，最终一致性可接受 |
| 完整的能力探测协议 | 简单版本号足够，不需要复杂协商 |

> **设计原则**：这是本地单用户工具，不是分布式系统。
> 企业级的可靠性保障在当前场景是过度设计。

---

## 12. 开放问题

1. **CLI 工具 vs nc 直连**：claude_hook.sh 用 nc 直连 vimo-agent Socket，还是提供 `vimo-notify` CLI？
2. **HookEvent 持久化**：是否需要持久化 HookEvent 供回放？
3. **多实例场景**：多个 Claude Code 实例同时运行时的处理？
