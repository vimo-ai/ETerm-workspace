# Permission Request ToolUseId Correlator

> 创建时间: 2026-02-11
> 状态: 设计完成，待实现
> 关联 session: `333fb989`(分析+录制系统) → 待定(实现)

## 1. 问题

iOS 远程审批 Claude Code 工具权限时，**审批结果无法关联到具体的 tool call**。

根因：Claude Code Hooks API **by design 不在 PermissionRequest 事件中提供 `tool_use_id`**。

官方文档原文：
> "PermissionRequest hooks receive `tool_name` and `tool_input` fields like PreToolUse hooks, but without `tool_use_id`"

## 2. Claude Code 工具调用生命周期

Claude Code 是**阻塞模型**——一次只能有一个工具在等权限审批。

```
PreToolUse       ← tool_name ✅  tool_use_id ✅
  ↓ (紧邻, <1ms)
PermissionRequest ← tool_name ✅  tool_use_id ❌  (API 不提供)
  ↓ (用户审批后)
PostToolUse      ← tool_name ✅  tool_use_id ✅
```

录制数据验证（来自 `~/.vimo/recordings/` JSONL）：

```json
// PreToolUse 紧接着 PermissionRequest，同一秒内
{"ts":"...T11:23:45.123Z","event":"PreToolUse","raw":{"tool_name":"Write","tool_use_id":"toolu_xxx",...}}
{"ts":"...T11:23:45.124Z","event":"PermissionRequest","raw":{"tool_name":"Write",...}}  // 无 tool_use_id
```

## 3. 当前链路分析

### 3.1 两条上游路径

```
                    Claude Code Hook
                          ↓
                    claude_hook.sh
                     ↓          ↓
               agent.sock    claude.sock
               (vimo-agent)  (ETerm)
                     ↓          ↓
               vlaude-daemon  VlaudePlugin
                     ↓          ↓
                     Server (汇聚点)
                          ↓
                        iOS
```

### 3.2 ETerm 路径（活跃）

```
Hook PermissionRequest (无 tool_use_id)
  → claude_hook.sh → claude.sock
  → AICliKitPlugin.handlePermissionRequest()
  → VlaudePlugin.handleClaudePermissionPrompt()
  → VlaudeClient.emitPermissionRequest(toolUseId: "")  ← 空字符串
  → Server (daemon:permissionRequest) → iOS (approval-request)
```

关键文件：
- `ETerm/Plugins/AICliKit/Sources/AICliKit/Providers/ClaudeProvider.swift` — 事件解析
- `ETerm/Plugins/AICliKit/Sources/AICliKit/AICliKitPlugin.swift:393-408` — 分发 permissionRequest
- `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/VlaudePlugin.swift:637-677` — handleClaudePermissionPrompt
- `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/VlaudeClient.swift:481-522` — emitPermissionRequest

### 3.3 Daemon 路径（权限请求部分为死代码）

```
Hook PermissionRequest
  → claude_hook.sh → agent.sock
  → vimo-agent: 只触发 JSONL 采集，忽略 tool_name/tool_use_id
  → vlaude-daemon-rs: request_approval() 存在但无调用者
  → ❌ 权限请求到不了 Server/iOS
```

关键发现：
- `ai-cli-session-db/src/agent/handler.rs:209-232` — handle_hook_event 只做 trigger_collect
- `vlaude-core/daemon-logic/src/service.rs:1428-1480` — request_approval() 是公开方法，但**零调用**
- 事件名不匹配 bug：daemon 匹配 `"server:approvalResponse"`，server 发 `"server:permissionResponse"`

### 3.4 Server 层（纯透传）

```typescript
// DaemonGateway (daemon.gateway.ts:786)
handleApprovalRequest(data) {
    this.eventEmitter.emit('app.sendApprovalRequest', data);
}

// AppGateway (app.gateway.ts:1024)
handleSendApprovalRequestEvent(data) {
    this.server.emit('approval-request', {
        requestId: data.requestId,
        sessionId: data.sessionId,
        toolName: data.toolName,
        input: data.input,
        toolUseID: data.toolUseID,  // 有什么传什么
        description: data.description,
    });
}
```

Server 不做任何加工，原样转发。

## 4. 方案：Server 层 ToolUseCorrelator

### 4.1 设计原则

- **两条上游路径一致**：ETerm 和 daemon 都是转发原始事件到 Server
- **上游是哑管道**：不做关联逻辑，只负责原样转发
- **Server 是智能层**：关联、enrichment、路由
- **一处实现**：TypeScript，覆盖所有上游

### 4.2 需要的改动

#### 上游：转发 PreToolUse 事件（ETerm + Daemon）

目前 ETerm 只转发 PermissionRequest 到 Server，不转发 PreToolUse。

需要新增事件：`daemon:preToolUse`

```typescript
// 新增 payload
interface PreToolUsePayload {
    sessionId: string;
    toolName: string;
    toolUseId: string;
    toolInput: any;
}
```

ETerm (VlaudePlugin)：收到 PreToolUse 时，额外 emit `daemon:preToolUse` 到 Server。
Daemon：同理（daemon 路径权限请求功能激活时一并实现）。

#### Server：DaemonGateway 新增 Correlator

```typescript
class ToolUseCorrelator {
    // session_id → 最近的 PreToolUse 上下文
    private pending: Map<string, {
        toolUseId: string;
        toolName: string;
        timestamp: number;
    }> = new Map();

    /** PreToolUse 到达时缓存 */
    onPreToolUse(sessionId: string, toolUseId: string, toolName: string): void {
        this.pending.set(sessionId, {
            toolUseId,
            toolName,
            timestamp: Date.now(),
        });
    }

    /** PermissionRequest 到达时关联 */
    resolveToolUseId(sessionId: string, toolName: string): string | null {
        const ctx = this.pending.get(sessionId);
        if (!ctx) return null;

        // 校验：tool_name 必须匹配 + 5s 时间窗口
        if (ctx.toolName !== toolName || Date.now() - ctx.timestamp > 5000) {
            this.pending.delete(sessionId);
            return null;
        }

        // 一次性消费
        this.pending.delete(sessionId);
        return ctx.toolUseId;
    }

    /** PostToolUse / SessionEnd 清理 */
    cleanup(sessionId: string): void {
        this.pending.delete(sessionId);
    }
}
```

#### Server：DaemonGateway 集成

```typescript
// 新增 handler
@SubscribeMessage(DaemonEvents.PRE_TOOL_USE)
handlePreToolUse(client: Socket, data: PreToolUsePayload) {
    this.correlator.onPreToolUse(data.sessionId, data.toolUseId, data.toolName);
}

// 修改现有 handler
@SubscribeMessage(DaemonEvents.PERMISSION_REQUEST)
handleApprovalRequest(client: Socket, data: ApprovalRequestPayload) {
    // 关联 toolUseId
    if (!data.toolUseID) {
        const resolved = this.correlator.resolveToolUseId(data.sessionId, data.toolName);
        if (resolved) {
            data.toolUseID = resolved;
        }
    }
    this.eventEmitter.emit('app.sendApprovalRequest', data);
}
```

### 4.3 严谨性保证

| 保证 | 机制 |
|------|------|
| 不误匹配 | tool_name 必须一致 |
| 不过期匹配 | 5s 时间窗口 |
| 不重复匹配 | 一次性消费（resolve 后删除） |
| 不泄漏内存 | PostToolUse/SessionEnd 兜底清理 |
| 兼容已有 toolUseId | `if (!data.toolUseID)` 判断，上游已提供则不覆盖 |

### 4.4 PreToolUse 转发的额外价值

除了支持 correlator，PreToolUse 事件转发到 iOS 还能支持：
- 工具执行状态指示（spinner / "正在读取文件..."）
- Timeline 中展示完整工具生命周期（开始 → 权限 → 完成）
- 未来的性能统计（工具执行耗时）

## 5. 实现步骤

### Phase 1：Server Correlator + ETerm PreToolUse 转发

1. **Server**: 新增 `ToolUseCorrelator` 类
2. **Server**: DaemonGateway 新增 `daemon:preToolUse` handler
3. **Server**: 修改 `handleApprovalRequest` 集成 correlator
4. **ETerm (VlaudePlugin)**: 收到 PreToolUse 时 emit `daemon:preToolUse`
5. **验证**: 用录制系统回放 PreToolUse + PermissionRequest，确认 iOS 收到完整 toolUseId

### Phase 2：iOS 审批匹配修复

6. **iOS (SessionDetailViewModel)**: 用 toolUseId 精确匹配 pending approval 到具体 tool_use 事件
7. **清理**: 移除 `unmatchedApprovals` 等临时 workaround

### Phase 3（后续）：Daemon 路径激活

8. **Daemon**: 激活 `request_approval()` 调用链
9. **Daemon**: 修复事件名 `"server:approvalResponse"` → `"server:permissionResponse"`
10. **Daemon**: 转发 PreToolUse 到 Server

## 6. 发现的 Bug（待修复）

### 6.1 Daemon 事件名不匹配

- **位置**: `vlaude-core/daemon-logic/src/service.rs:595`
- **问题**: 匹配 `"server:approvalResponse"`，但 server 发的是 `"server:permissionResponse"`
- **影响**: daemon 路径的审批响应永远收不到（当前是死代码所以没暴露）
- **修复**: 改为 `"server:permissionResponse"` 或使用 `server_events::PERMISSION_RESPONSE` 常量

### 6.2 Daemon request_approval() 无调用者

- **位置**: `vlaude-core/daemon-logic/src/service.rs:1428`
- **问题**: 公开方法但零调用，HookEvent handler 不转发 PermissionRequest
- **影响**: ETerm 未运行时权限请求无法到达 iOS
- **修复**: Phase 3 实现

## 7. 已完成的基础设施

### 7.1 Hook 事件录制系统

- `claude_hook.sh` 已接入录制（所有事件写入 `~/.vimo/recordings/{session_id}.jsonl`）
- `scripts/replay-hooks.sh` 回放工具（支持 --target hook|eterm, --event 过滤, --list 列出）
- `scripts/list-recordings.sh` 查看录制概览
- settings.json 已补全 PreToolUse/PostToolUse 的 hook 覆盖

### 7.2 iOS 端已完成的修复

- bufferMessage debounce → throttle（修复流式更新卡顿）
- 移除 TurnCard 中重复的 pendingApprovalCard（CompactToolRow 已有审批按钮）
- unmatchedApprovals 临时方案（Phase 2 中会被 toolUseId 精确匹配替代）
