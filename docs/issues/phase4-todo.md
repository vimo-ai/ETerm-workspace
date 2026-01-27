# Phase 4 问题清单

## 发现日期: 2026-01-14

---

## 问题 1: `getSessionBySessionId` 设计缺陷

**现象**: iOS 进入 session 详情页，`getSessionDetail` 返回 null，导致 `sendMessage` 链路断掉

**根因**: `requestSessionBySessionId` 实现绕圈子
```typescript
// daemon.gateway.ts:308-314
async requestSessionBySessionId(sessionId: string, projectPath: string) {
    const result = await this.requestSessions(projectPath, 1000, 0);  // 获取整个列表
    const session = result.sessions.find((s) => s.sessionId === sessionId);  // 再筛选
    return session || null;
}
```

**应该**: 直接向 Daemon 请求单个 session 详情
- 新增 `server:requestSessionDetail` 事件
- Daemon 直接根据 sessionId 返回

**影响文件**:
- `vlaude-server/src/module/daemon-gateway/daemon.gateway.ts`
- `vlaude-core/daemon-logic/src/service.rs`
- `vlaude-core/socket-client/src/events.rs`

---

## 问题 2: iOS `sendMessage` 不必要的依赖

**现象**: `sendMessage` guard 了 `session` 对象，但实际没用到

**代码位置**: `Vlaude/ViewModels/SessionDetailViewModel.swift:488-501`

**临时修复**: 已删除多余的 guard（但这是 workaround，不是根本解决）

**根本解决**: 修复问题 1 后，`session` 对象能正常获取

---

## 问题 3: Server DB 数据同步断了

**现象**: Server DB (MySQL) 最新数据停在 2025-12-30，15 天没有新数据同步

**数据状态**:
- Projects: 111
- Sessions: 2311
- Messages: 238（比例很低）

**待查原因**:
- Daemon → Server 推送链路是否正常
- sync 模式是否正确配置和触发

---

## 问题 4: Server DB 应该用 SQLite

**现象**: 当前 Server 用 MySQL，但设计上应该和 Daemon DB 保持一致用 SQLite

**位置**: `vlaude-server/prisma/schema.prisma` - `provider = "mysql"`

**TODO**: 评估是否迁移到 SQLite，或保持 MySQL

---

## 问题 5: forward/sync 模式行为不一致

**现象**:
- `getSessionMessages` 有 sync 模式 DB fallback
- `getSessionBySessionId` 没有 sync 模式逻辑，只走 Daemon

**设计原则** (ARCHITECTURE_DATA_SYNC.md):
> Server 响应拉取：并行读 DB + 请求 Daemon，Daemon 为重

**TODO**: 统一所有 API 的 forward/sync 模式行为

---

## 问题 6: `list_projects` 直接扫描文件系统，绕过 DB ✅ 已修复

**现象**: Daemon 的 `list_projects` 直接扫描 `~/.claude/projects/` 目录
```rust
// ai-cli-session-db/src/reader.rs:116
let entries = match fs::read_dir(&self.projects_path)
```

**问题**:
- 绕过了 Daemon DB
- 无法用 SQL 分页（`LIMIT OFFSET`）
- 每次都要扫描整个目录

**设计原则** (ARCHITECTURE_DATA_SYNC.md):
```
JSONL → 文件监听 → 解析 → 写入 Daemon DB
                              ↓
                    查询时从 DB 读取
```

**修复内容** (2026-01-14):
- `service.rs`: `handle_request_project_data` 改用 `shared_db.list_projects_with_stats()`
- `service.rs`: `handle_request_session_metadata` 改用 `shared_db.list_sessions_by_project_path()`
- `types.rs`: 新增 `SessionWithProject` 结构，包含 `projectName`/`projectPath`
- `db.rs`: `list_sessions_by_project_path` 返回 `SessionWithProject`（JOIN projects 表）
- 保留 `reader` 作为 fallback（当 `shared_db` 未初始化时）

---

## 问题 7: 列表查询没有分页，一次性拉 1000 条

**现象**: Projects 和 Sessions 列表都一次性拉 1000 条
```typescript
// daemon.gateway.ts
const result = await this.requestProjects(1000, 0);           // 253行
const result = await this.requestSessions(projectPath, 1000, 0);  // 309行
```

**问题**:
- iOS 列表页只显示 20 条，应该按需分页
- 浪费带宽和内存，增加延迟
- Daemon 返回大量数据后 Server 再切片，完全没必要

**应该**:
- iOS 请求时传 limit/offset
- Server 透传给 Daemon
- Daemon 直接返回分页后的数据

**影响文件**:
- `daemon.gateway.ts` - `requestProjects`, `requestSessions`
- iOS 端列表请求逻辑

---

## 问题 8: VlaudeKit 架构改造 ✅ 已完成

**现象**: iOS session 列表查询耗时 ~1057ms，其中 `sessionReader.listSessions()` 占 ~1046ms

**已完成**:
- [x] vlaude-ffi 在 build.sh 中编译部署
- [x] VlaudeKit 改用 vlaude-ffi
- [x] agent session 过滤统一到 DB 层 SQL (2026-01-15)

**关联问题已修复** (2026-01-15):
- [x] vlaude-ffi 改为调用 daemon-logic（问题 9 已修复）
- [x] daemon-logic 数据源统一到 DB（问题 10 已修复）
- [x] 主动采集 + 增量推送机制（问题 11 已修复）

---

## 问题 9: vlaude-ffi 绕过 daemon-logic ✅ 已修复

**发现日期**: 2026-01-15
**修复日期**: 2026-01-15

**现象**: vlaude-ffi 直接调用 SharedDB，绕过了 daemon-logic

**设计意图**:
```
VlaudeKit (Swift) → vlaude-ffi → daemon-logic → SharedDB
daemon-rs         →              daemon-logic → SharedDB
                                      ↑
                              统一的业务逻辑层
```

**修复内容**:
1. 新增 `daemon-logic/src/sync_api.rs` - 同步 API 模块
2. vlaude-ffi 改为调用 `daemon_logic::sync_api::*` 方法

**修复后架构**:
```
VlaudeKit → vlaude-ffi → daemon_logic::sync_api → SharedDB
daemon-rs →              daemon_logic::service  → SharedDB
                                 ↑
                         统一的业务逻辑层
```

| 函数 | 修复后调用 |
|------|----------|
| `vlaude_list_projects` | `daemon_logic::sync_api::list_projects()` |
| `vlaude_list_sessions` | `daemon_logic::sync_api::list_sessions()` |
| `vlaude_get_messages` | `daemon_logic::sync_api::get_messages()` |
| `vlaude_search` | `daemon_logic::sync_api::search()` |
| `vlaude_get_stats` | `daemon_logic::sync_api::get_stats()` |

---

## 问题 10: daemon-logic 数据源不统一 ✅ 已修复

**发现日期**: 2026-01-15
**修复日期**: 2026-01-15

**现象**: daemon-logic 内部混用 reader (JSONL) 和 shared_db (DB)

**修复后状态**:

| 方法 | 数据源 | 状态 |
|------|--------|------|
| `handle_request_project_data` | shared_db ✅ | 已正确 |
| `handle_request_session_metadata` | shared_db ✅ | 已正确 |
| `handle_request_session_messages` | shared_db ✅ | 已修复 |
| 启动时 `push_initial_data` | shared_db ✅ | 已修复 |

**设计原则**:
```
JSONL (真相源)
    ↓ 文件监听 + 解析
Daemon DB (查询数据源)
    ↓
所有查询从 DB 读取
```

**修复内容**:
- `handle_request_session_messages` 改为从 DB 读取消息
- `push_initial_data` 改为从 DB 读取项目和会话列表
- DB 层 `list_messages_ordered` 支持排序参数

---

## 问题 11: 缺少主动采集 + 增量推送机制 ✅ 已修复

**发现日期**: 2026-01-15
**修复日期**: 2026-01-15

**设计流程**:
```
iOS 请求 session messages
    │
    ├──→ 1. 从 DB 读取 → 立即返回（快速响应）
    │
    └──→ 2. 同时触发 JSONL 采集（异步）
              │
              ↓
         采集完成，对比 DB
              │
              ├── 有新数据 → 增量推送给 iOS
              │
              └── 无新数据 → 不做任何事
```

**修复内容**:
1. `handle_request_session_messages` 先从 DB 读取并立即返回
2. 异步调用 `trigger_session_sync()` 触发 JSONL 采集
3. `do_session_sync()` 对比 JSONL 和 DB 消息数，发现增量则通过 socket 推送

**新增方法** (`daemon-logic/src/service.rs`):
- `trigger_session_sync()` - 异步触发 JSONL 采集
- `do_session_sync()` - 执行 JSONL → DB 对比，推送增量

---

## 优先级建议

### 待处理
1. **P1**: 问题 1 - getSessionBySessionId 设计缺陷
2. **P1**: 问题 5 - forward/sync 模式行为不一致
3. **P2**: 问题 3 - 数据同步
4. **P2**: 问题 4 - DB 选型
5. **P2**: 问题 7 - 列表查询没有分页

### 已完成
6. **已临时修复**: 问题 2 - iOS sendMessage 不必要的依赖
7. **✅ 已修复**: 问题 6 - Daemon 查询改走 DB
8. **✅ 已修复**: 问题 8 - VlaudeKit 改用 vlaude-ffi
9. **✅ 已修复**: 问题 9 - vlaude-ffi 改为调用 daemon-logic (2026-01-15)
10. **✅ 已修复**: 问题 10 - daemon-logic 数据源统一到 DB (2026-01-15)
11. **✅ 已修复**: 问题 11 - 主动采集 + 增量推送机制 (2026-01-15)

---

## 相关文件

- 架构设计: `ARCHITECTURE_DATA_SYNC.md`
- Server: `vlaude-server/src/module/daemon-gateway/daemon.gateway.ts`
- Server: `vlaude-server/src/module/session/session.service.ts`
- Daemon: `vlaude-core/daemon-logic/src/service.rs`
- Daemon: `vlaude-core/daemon-logic/src/shared_db.rs`
- Daemon: `vlaude-core/daemon-logic/src/sync_api.rs` (新增)
- FFI: `vlaude-core/vlaude-ffi/src/lib.rs`
- DB: `ai-cli-session-db/src/db.rs`
- DB: `ai-cli-session-db/src/types.rs`
- iOS: `Vlaude/ViewModels/SessionDetailViewModel.swift`
