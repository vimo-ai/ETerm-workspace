# Redis 状态同步改造计划

## 背景

当前 ETerm 在线状态存储在 vlaude-server 内存中，导致：
- Server 重启后状态丢失
- 各端断线重连后状态不一致
- 缺乏可靠的状态同步机制

## 目标架构

```
┌─────────────────────────────────────────────────────────────────────────┐
│                                Redis                                     │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  Keys:                                                           │    │
│  │    vlaude:services:daemon:<id>   ← VlaudeKit / vlaude-daemon-rs │    │
│  │    vlaude:services:server:<addr> ← Server                        │    │
│  │                                                                  │    │
│  │  Channel:                                                        │    │
│  │    vlaude:channel:service-registry  ← Pub/Sub 状态广播           │    │
│  └─────────────────────────────────────────────────────────────────┘    │
└──────────────────────────────┬──────────────────────────────────────────┘
                               │
         ┌─────────────────────┼─────────────────────┐
         │                     │                     │
         ▼                     ▼                     ▼
┌─────────────────┐   ┌─────────────────┐   ┌─────────────────┐
│   VlaudeKit     │   │     Server      │   │ vlaude-daemon-rs│
│   (ETerm插件)   │   │    (NestJS)     │   │   (后台进程)    │
├─────────────────┤   ├─────────────────┤   ├─────────────────┤
│ socket-client   │   │ 订阅 Redis      │   │ socket-client   │
│ (via FFI)       │   │ 分发给 App      │   │ (直接依赖)      │
└────────┬────────┘   └────────┬────────┘   └────────┬────────┘
         │                     │                     │
         │      Socket.IO      │      Socket.IO      │
         │    (/daemon ns)     │    (/daemon ns)     │
         └─────────────────────┼─────────────────────┘
                               │
                               ▼
                      ┌─────────────────┐
                      │   Vlaude App    │
                      │     (iOS)       │
                      ├─────────────────┤
                      │ 只连 Server     │
                      │ 不连 Redis      │
                      └─────────────────┘
```

## 核心原则

1. **状态持久化**：所有状态存 Redis，不存内存
2. **TTL 自动过期**：节点下线通过 TTL 过期检测
3. **Pub/Sub 广播**：状态变化通过 Redis Channel 广播
4. **Server 分发**：Vlaude App 的通知由 Server 分发
5. **共享底层**：VlaudeKit 和 vlaude-daemon-rs 共用 socket-client

## 状态同步流程

### 节点上线

```
VlaudeKit/daemon-rs 启动
    │
    ├─→ 连接 Redis
    │
    ├─→ 从 Redis 发现 Server 地址 (get_servers)
    │
    ├─→ 注册自己到 Redis (register + TTL)
    │      Key: vlaude:services:daemon:<device-id>
    │      Value: { address, sessions: [], ... }
    │
    ├─→ 发布 online 事件到 Pub/Sub
    │
    ├─→ 连接 Server (Socket.IO)
    │
    └─→ 启动心跳续期 (keep_alive)
```

### 节点下线

```
正常下线:
    VlaudeKit/daemon-rs 退出
        │
        ├─→ 从 Redis 注销 (unregister)
        │
        └─→ 发布 offline 事件到 Pub/Sub

异常下线:
    进程崩溃/网络断开
        │
        └─→ Redis TTL 自动过期 (60s)
              │
              └─→ Server 轮询检测或 keyspace notification
```

### Vlaude App 获取状态

```
App 连接 Server
    │
    ├─→ 调用 app:queryEtermStatus
    │
    └─→ Server 从 Redis 读取 daemon 列表返回
```

### 状态变化通知

```
Daemon 状态变化
    │
    ├─→ 更新 Redis Key
    │
    ├─→ 发布事件到 Pub/Sub
    │
    └─→ Server 订阅 Pub/Sub
          │
          └─→ 广播给 Vlaude App (Socket.IO)
```

---

## 改造任务

### Phase 1: socket-client 层改造 ✅ 完成

#### 1.1 扩展 ServiceRegistry 功能

**文件**: `vlaude/packages/vlaude-core/socket-client/src/registry.rs`

- [x] 添加 daemon 专用注册方法
- [x] 添加 session 状态存储
- [x] 添加 daemon 列表查询
- [x] 添加 keyspace notification 支持（检测 TTL 过期）

#### 1.2 集成到 SocketClient

**文件**: `vlaude/packages/vlaude-core/socket-client/src/client.rs`

- [x] 添加 ServiceRegistry 字段
- [x] 连接时自动发现 Server 地址
- [x] 连接成功后自动注册到 Redis
- [x] 启动心跳续期任务
- [x] 断开时自动注销
- [x] `start_reconnect_loop()` - 监听 Server online 事件后自动重连

#### 1.3 暴露 FFI 接口

**文件**: `vlaude/packages/vlaude-core/socket-client-ffi/src/lib.rs`

- [x] `socket_client_init_registry` - 初始化 Redis 连接
- [x] `socket_client_discover_server` - 发现 Server 地址
- [x] `socket_client_register_daemon` - 注册 daemon
- [x] `socket_client_update_sessions` - 更新 session 列表
- [x] `socket_client_unregister` - 注销

---

### Phase 2: VlaudeKit 改造 ✅ 完成

#### 2.1 SocketClientBridge 扩展

**文件**: `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/SocketClientBridge.swift`

- [x] 添加 Redis 初始化方法
- [x] 添加服务发现方法
- [x] 添加 daemon 注册/注销方法
- [x] 添加 session 状态更新方法
- [x] 新增 `socketClientDidReconnect` 和 `reconnectFailed` 方法

#### 2.2 VlaudeClient 改造

**文件**: `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/VlaudeClient.swift`

- [x] 启动时先初始化 Redis
- [x] 通过 Redis 发现 Server 地址（移除硬编码配置）
- [x] 连接 Server 后注册到 Redis
- [x] session 变化时更新 Redis
- [x] 断开时注销
- [x] 实现重连代理，重连成功后重新发送 register、reportOnline 并推送初始数据

#### 2.3 VlaudePlugin 适配

**文件**: `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/VlaudePlugin.swift`

- [x] 适配新的连接流程
- [x] 处理 Redis 连接失败的降级逻辑

---

### Phase 3: Server 改造 ✅ 完成

#### 3.1 DaemonGateway 改造

**文件**: `vlaude/packages/vlaude-server/src/module/daemon-gateway/daemon.gateway.ts`

- [x] 移除内存状态 (`etermOnline`, `etermSessions`)
- [x] 注入 RegistryService
- [x] `isEtermOnline()` → 改为 async，从 Redis 读取
- [x] `isSessionInEterm()` → 改为 async，从 Redis 读取
- [x] `getEtermSessions()` → 改为 async，从 Redis 读取
- [x] `getEtermSessionCounts()` → 改为 async，从 Redis 读取
- [x] `injectMessageToEterm()` → 改为 async
- [x] `requestEtermCreateSession()` → 改为 async
- [x] `notifyEtermMobileViewing()` → 改为 async
- [x] 事件处理器不再更新内存状态，只转发事件

#### 3.1.1 模块依赖修复

**文件**: `vlaude/packages/vlaude-server/src/module/daemon-gateway/daemon-gateway.module.ts`

- [x] 导入 `RegistryModule`，解决 `RegistryService` 依赖注入错误

#### 3.1.2 Controller 调用方修复（Codex 验证发现）

**文件**: `vlaude/packages/vlaude-server/src/module/session/session.controller.ts`

- [x] `serializeSession()` → 改为 async，await `isSessionInEterm()`
- [x] `serializeSessions()` → 改为 async，使用 `Promise.all`
- [x] `getSessionsByPath()` → await `serializeSessions()` 和 `isEtermOnline()`
- [x] `getSessionBySessionId()` → await `serializeSession()`
- [x] `getSessionsByProject()` → await `serializeSessions()`
- [x] `getSessionById()` → await `serializeSession()`

**文件**: `vlaude/packages/vlaude-server/src/module/project/project.controller.ts`

- [x] `getAllProjects()` → await `isEtermOnline()` 和 `getEtermSessions()`

#### 3.2 RegistryService 扩展

**文件**: `vlaude/packages/vlaude-server/src/module/registry/registry.service.ts`

- [x] `getDaemons()` 方法已实现
- [x] `getDaemon(deviceId)` 方法已实现
- [x] 订阅 Redis Pub/Sub，通过 EventEmitter 通知 AppGateway

#### 3.3 AppGateway 适配

**文件**: `vlaude/packages/vlaude-server/src/gateway/app.gateway.ts`

- [x] `handleQueryEtermStatus` → 改为 async，从 Redis 读取
- [x] `handleSessionSubscribe` → 改为 await 调用
- [x] `handleSessionUnsubscribe` → 改为 async，await 调用
- [x] `handleMessageSend` → await 调用
- [x] 监听 RegistryService 事件，广播给 Vlaude App

---

### Phase 4: Vlaude App 适配 ✅ 完成（无需修改）

#### 4.1 状态查询适配

**文件**: `vlaude/packages/Vlaude/Vlaude/Services/WebSocketManager.swift`

- [x] `queryEtermStatus` 正常工作 - Server 返回格式不变
- [x] ETerm 状态事件格式不变：`eterm:statusChanged`, `eterm:sessionAvailable`, `eterm:sessionUnavailable`, `eterm:sessionCreated`
- [x] Daemon 状态事件格式不变：`daemon:online`, `daemon:offline`, `daemon:sessionUpdate`

#### 4.2 断线重连

- [x] Socket.IO 内置自动重连机制
- [x] 重连成功后自动调用 `queryEtermStatus()` 和 `queryDaemons()` 刷新状态

**无需修改原因**：iOS 端只是消费者，不关心 Server 内部是从内存还是 Redis 读取数据，返回的数据结构完全相同。

---

## Redis 数据结构设计

### Daemon 注册 Key

```
Key: vlaude:services:daemon:<device-id>
TTL: 60s
Value: {
  "deviceId": "mac-xxx",
  "deviceName": "My Mac",
  "platform": "darwin",
  "version": "1.0.0",
  "address": "192.168.1.100",  // 可选，用于直连
  "sessions": [
    {
      "sessionId": "uuid-xxx",
      "projectPath": "/path/to/project"
    }
  ],
  "registeredAt": 1704326400000
}
```

### Server 注册 Key

```
Key: vlaude:services:server:<address>
TTL: 60s
Value: {
  "address": "localhost:10005",
  "ttl": 60,
  "registeredAt": 1704326400000
}
```

### Pub/Sub Channel

```
Channel: vlaude:channel:service-registry
Message: {
  "type": "online" | "offline" | "session_update",
  "service": "daemon" | "server",
  "deviceId": "mac-xxx",  // daemon only
  "address": "localhost:10005",  // server only
  "sessions": [...],  // session_update only
  "timestamp": 1704326400000
}
```

---

## 测试场景

### 场景 1: Server 重启

1. ETerm 在线，Vlaude App 连接中
2. Server 重启
3. Server 启动后从 Redis 读取 ETerm 状态
4. Vlaude App 重连后看到 ETerm 在线

### 场景 2: ETerm 正常退出

1. ETerm 退出
2. VlaudeKit 注销 Redis
3. Server 收到 Pub/Sub offline 事件
4. Server 广播给 Vlaude App
5. Vlaude App 显示 ETerm 离线

### 场景 3: ETerm 崩溃

1. ETerm 崩溃（无法注销）
2. Redis TTL 60s 后过期
3. Server 检测到过期（轮询或 keyspace notification）
4. Server 广播给 Vlaude App

### 场景 4: Vlaude App 查询状态

1. Vlaude App 连接 Server
2. 调用 `app:queryEtermStatus`
3. Server 从 Redis 读取 daemon 列表
4. 返回给 Vlaude App

---

## 风险与降级

### Redis 不可用

- socket-client 连接 Redis 失败时，降级到直连模式
- 使用配置的 Server 地址
- 状态同步功能不可用，但基本功能正常

### Server 不可用

- Vlaude App 显示断线状态
- 用户手动重连
- VlaudeKit/daemon 继续心跳续期 Redis

---

## 时间估计

| Phase | 任务 | 估计 |
|-------|------|------|
| Phase 1 | socket-client 层改造 | - |
| Phase 2 | VlaudeKit 改造 | - |
| Phase 3 | Server 改造 | - |
| Phase 4 | Vlaude App 适配 | - |
| - | 测试与调试 | - |

---

## 相关文件

- `vlaude/packages/vlaude-core/socket-client/src/registry.rs`
- `vlaude/packages/vlaude-core/socket-client/src/client.rs`
- `vlaude/packages/vlaude-core/socket-client-ffi/src/lib.rs`
- `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/SocketClientBridge.swift`
- `ETerm/Plugins/VlaudeKit/Sources/VlaudeKit/VlaudeClient.swift`
- `vlaude/packages/vlaude-server/src/module/daemon-gateway/daemon.gateway.ts`
- `vlaude/packages/vlaude-server/src/module/daemon-gateway/daemon-gateway.module.ts`
- `vlaude/packages/vlaude-server/src/module/registry/registry.service.ts`
- `vlaude/packages/vlaude-server/src/module/session/session.controller.ts`
- `vlaude/packages/vlaude-server/src/module/project/project.controller.ts`
- `vlaude/packages/vlaude-server/src/gateway/app.gateway.ts`
