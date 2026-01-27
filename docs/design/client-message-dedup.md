# clientMessageId 消息去重方案

## 背景问题

iOS Vlaude App 发送消息时出现重复：
1. **乐观更新**：iOS 立即显示用户消息（生成 UUID: `aaa-111`）
2. **服务器推送**：ETerm 检测到 JSONL 变化后推送消息（JSONL 原始 UUID: `bbb-222`）
3. **去重失败**：因为 UUID 不同，iOS 无法识别是同一条消息

## 解决方案

使用 `clientMessageId` 作为幂等键，由消息发起方（iOS）生成，全链路透传。

```
发送: { text, sessionId, clientMessageId }  ← iOS 生成
推送: { uuid, type, clientMessageId, ... }  ← ETerm 携带
去重: pendingMessages[clientMessageId] → 替换为真实消息
```

## 架构总览

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                              iOS Vlaude App                                   │
│  ┌─────────────────────────┐    ┌─────────────────────────────────────────┐  │
│  │ SessionDetailViewModel  │    │         WebSocketManager                │  │
│  │ - pendingMessages map   │───▶│ - sendMessage(text, sessionId,         │  │
│  │ - 生成 clientMessageId   │    │              clientMessageId)          │  │
│  └─────────────────────────┘    └─────────────────────────────────────────┘  │
└───────────────────────────────────────┬──────────────────────────────────────┘
                                        │ Socket.IO (/ namespace)
                                        │ message:send {sessionId, text, clientMessageId}
                                        ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                            vlaude-server (NestJS)                             │
│  ┌─────────────────────────┐    ┌─────────────────────────────────────────┐  │
│  │    app.gateway.ts       │    │        daemon.gateway.ts                │  │
│  │ - handleMessageSend     │───▶│ - injectMessageToEterm                  │  │
│  │ - 透传 clientMessageId   │    │ - 广播 server:injectToEterm             │  │
│  └─────────────────────────┘    └─────────────────────────────────────────┘  │
└───────────────────────────────────────┬──────────────────────────────────────┘
                                        │ Socket.IO (/daemon namespace)
                                        │ server:injectToEterm {sessionId, text, clientMessageId}
                                        ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                           ETerm VlaudeKit (Swift)                             │
│  ┌─────────────────────────┐    ┌─────────────────────────────────────────┐  │
│  │    VlaudeClient.swift   │    │        SocketClientBridge.swift         │  │
│  │ - delegate 回调          │◀───│ - 事件回调 (Rust FFI wrapper)            │  │
│  │ - pushMessage 携带 id   │    │ - notifyNewMessage (JSON 透传)          │  │
│  └─────────────────────────┘    └─────────────────────────────────────────┘  │
│              │                                    ▲                           │
│              ▼                                    │                           │
│  ┌─────────────────────────┐    ┌─────────────────────────────────────────┐  │
│  │   VlaudePlugin.swift    │    │       socket-client-ffi (Rust)          │  │
│  │ - pendingClientMsgIds   │    │ - JSON 透传，不需要改动                   │  │
│  │ - 存储/查找 clientMsgId │    │ - socket_client_notify_new_message      │  │
│  └─────────────────────────┘    └─────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────┘
```

## 数据流详解

### 下行链路（iOS → ETerm）

```
① iOS SessionDetailViewModel.sendMessage("hello")
   - 生成 clientMessageId = UUID().uuidString
   - pendingMessages[clientMessageId] = 乐观更新的 Message
   - wsManager.sendMessage("hello", sessionId, clientMessageId)

② iOS WebSocketManager.sendMessage
   - emit("message:send", {sessionId, text, clientMessageId})

③ Server app.gateway.ts handleMessageSend
   - 接收 {sessionId, text, clientMessageId}
   - isSessionInEterm(sessionId) → true
   - daemonGateway.injectMessageToEterm(sessionId, text, clientMessageId)

④ Server daemon.gateway.ts injectMessageToEterm
   - server.emit("server:injectToEterm", {sessionId, text, clientMessageId})

⑤ ETerm SocketClientBridge 收到事件回调
   - delegate.socketClient(didReceiveEvent: "server:injectToEterm", data: {...})

⑥ ETerm VlaudeClient.socketClient(didReceiveEvent:)
   - 解析 server:injectToEterm
   - delegate.vlaudeClient(didReceiveInject: sessionId, text, clientMessageId)

⑦ ETerm VlaudePlugin.didReceiveInject
   - pendingClientMessageIds[sessionId] = clientMessageId
   - 注入文本到终端
```

### 上行链路（ETerm → iOS）

```
⑧ ETerm SessionWatcher 检测 JSONL 文件变化
   - 解析新消息 [user_msg, assistant_msg, ...]
   - delegate.sessionWatcher(didReceiveMessages:)

⑨ ETerm VlaudePlugin.sessionWatcher(didReceiveMessages:)
   - 对于 user 类型消息：
     - clientMessageId = pendingClientMessageIds[sessionId]
     - pendingClientMessageIds.removeValue(forKey: sessionId)
   - client.pushMessage(sessionId, message, clientMessageId)

⑩ ETerm VlaudeClient.pushMessage
   - msgDict["uuid"] = message.uuid
   - msgDict["type"] = message.type
   - msgDict["clientMessageId"] = clientMessageId  // 关键：加入字典
   - socketBridge.notifyNewMessage(sessionId, msgDict)

⑪ SocketClientBridge.notifyNewMessage
   - JSON 序列化 msgDict
   - 调用 Rust FFI: socket_client_notify_new_message(sessionId, jsonStr)

⑫ Rust socket-client-ffi (JSON 透传，不需要改动)
   - emit "daemon:newMessage" {sessionId, message}

⑬ Server daemon.gateway.ts handleNewMessage
   - eventEmitter.emit('app.notifyNewMessage', {sessionId, message})

⑭ Server app.gateway.ts notifyNewMessage
   - emit("message:new", {sessionId, message})  // message 包含 clientMessageId

⑮ iOS WebSocketManager 收到 message:new
   - 解析 message，传给 SessionDetailViewModel

⑯ iOS SessionDetailViewModel.handleNewMessage
   - if let clientMsgId = message.clientMessageId:
       if pendingMessages[clientMsgId] != nil:
           → 找到匹配！用真实消息替换 pending 消息
           → pendingMessages.removeValue(forKey: clientMsgId)
           → return (不重复添加)
   - else:
       → 正常去重逻辑（用 uuid）
```

## 改动清单

| # | 文件 | 改动点 |
|---|------|--------|
| 1 | `Vlaude/Services/WebSocketManager.swift` | `sendMessage` 增加 `clientMessageId` 参数 |
| 2 | `Vlaude/ViewModels/SessionDetailViewModel.swift` | 生成 clientMessageId，pending map，匹配逻辑 |
| 3 | `vlaude-server/gateway/app.gateway.ts` | `handleMessageSend` 接收并透传 clientMessageId |
| 4 | `vlaude-server/daemon-gateway/daemon.gateway.ts` | `injectMessageToEterm` 广播时包含 clientMessageId |
| 5 | `VlaudeKit/VlaudeClient.swift` | delegate 方法增加 clientMessageId 参数 |
| 6 | `VlaudeKit/VlaudePlugin.swift` | 存储 pendingClientMessageIds，推送时携带 |

### 不需要改动

- **Rust FFI** (`socket-client-ffi/src/lib.rs`) - JSON 透传
- **Rust socket-client** (`socket-client/src/client.rs`) - JSON 透传
- **SocketClientBridge.swift** - 已有 `notifyNewMessage(message: [String: Any])`，支持任意字段

## 关键设计决策

### 1. 为什么由 iOS 生成 clientMessageId？

- iOS 是消息发起方，知道何时开始"等待"
- 避免服务端生成 ID 的网络延迟
- 与乐观更新时机一致

### 2. 为什么 Rust FFI 不需要改动？

Rust FFI 层是 JSON 透传设计：
```rust
let message: serde_json::Value = serde_json::from_str(msg_str)?;
handle.client.notify_new_message(sid, message).await
```
只要 Swift 在 message 字典中包含 `clientMessageId`，Rust 会原样传递。

### 3. clientMessageId 的生命周期

```
iOS 生成 → pendingMessages 存储
    ↓
Server 透传（不存储）
    ↓
ETerm 存储 → pendingClientMessageIds[sessionId]
    ↓
ETerm 推送时携带 → 清除 pendingClientMessageIds
    ↓
iOS 匹配后 → 清除 pendingMessages
```

### 4. 边界情况处理

| 场景 | 处理方式 |
|------|----------|
| 消息来自 ETerm 终端（非 iOS） | 没有 clientMessageId，走正常 uuid 去重 |
| iOS 发送失败 | pendingMessages 保留，可添加超时清理 |
| ETerm 推送丢失 | pendingMessages 保留，不影响功能 |
| 重复推送 | clientMessageId 已清除，走 uuid 去重 |

## 测试用例

1. **正常流程**：iOS 发送 → ETerm 收到 → 推送返回 → 去重成功
2. **ETerm 发送**：ETerm 终端发送 → 推送到 iOS → 正常显示（无 clientMessageId）
3. **网络延迟**：乐观更新显示 → 延迟收到推送 → 正确替换
4. **发送失败**：乐观更新显示 → 永不收到推送 → pending 保留（可加超时）

## 相关文件

- iOS: `vlaude/packages/Vlaude/Vlaude/`
- Server: `vlaude/packages/vlaude-server/src/`
- ETerm: `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/`
- Rust FFI: `vlaude/packages/vlaude-core/socket-client-ffi/`
