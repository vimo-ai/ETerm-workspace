# Memex 协作同步设计方案

> 创建时间: 2026-03-28
> 更新时间: 2026-04-20
> 状态: 设计中（架构已对齐）
> 关联文档: [session-chain-linking.md](./session-chain-linking.md), [agent-v2-rw-separation.md](./agent-v2-rw-separation.md)

---

## 1. 背景与目标

memex-rs 目前是纯本地系统：SQLite（ai-cli-session.db）+ FTS5 全文检索 + LanceDB 向量索引，数据存于 `~/.vimo/db/`，通过 axum 提供本地 HTTP API，vimo-agent 是唯一写入者（Unix Socket IPC）。

### 1.1 当前架构

```
vimo-agent (单写者)
     │ Unix Socket IPC
     ▼
ai-cli-session.db  ──── FTS5 全文索引
     │                        │
     ▼                        ▼
compact.db (L1-L3)      LanceDB (向量)
     │
     ▼
memex-rs axum HTTP  (仅本地访问)
```

### 1.2 目标功能

本次扩展需要支持三个新能力：

| 功能 | 说明 |
|------|------|
| **敏感词过滤** | 上传前过滤指定词汇，本地保留原始数据 |
| **文件夹过滤** | 白名单控制哪些项目参与同步 |
| **协作同步** | 自建服务器，项目级权限，多人共享会话历史 |

---

## 2. 核心问题分析

在开始设计之前，必须先解决六个关键技术问题。

### 问题 1：主键冲突

三张主表均使用 `INTEGER PRIMARY KEY AUTOINCREMENT`，各本地 DB 独立自增，不同客户端的 ID 天然冲突。

```
本地 A: projects.id = 1, 2, 3 ...
本地 B: projects.id = 1, 2, 3 ...   ← 同一个项目，ID 完全不同
```

**解决方案**：同步时不传 `id`，改用自然键：
- `projects` → `path`（有 UNIQUE 约束）
- `sessions` → `session_id`（有 UNIQUE 约束）
- `messages` → `uuid`（有 UNIQUE 约束，来自 Claude Code JSONL）

服务端接收后，按自然键 UPSERT，服务端本地自增 ID 与客户端完全隔离。

> **注意**：`messages.id` 用于 LanceDB 向量关联（`mark_messages_indexed`）和 FTS `content_rowid`，服务端独立重建即可。

### 问题 2：messages.uuid 去重

`uuid` 来自 Claude Code JSONL，全局唯一，每条消息一个。

- 同一会话不同用户：每人有独立的 Claude 会话，uuid 天然不同，无冲突
- 同一用户多设备：同一 JSONL 文件 uuid 相同，`ON CONFLICT(uuid) DO NOTHING` 完美处理

**结论**：uuid 去重策略无边界问题，直接复用现有约束。

### 问题 3：project.path 是本地绝对路径

```
Alice: /Users/alice/code/ETerm
Bob:   /Users/bob/dev/ETerm       ← 同一个 repo，路径不同
```

**解决方案**：新增 `projects.repo_url` 字段，采集时执行 `git remote get-url origin` 获取。服务端按 `repo_url` 归并项目，设置权限时以 repo 为粒度，而非本地路径。

对齐流程：
1. vimo-agent 采集时自动执行 `git remote get-url origin`，写入 `projects.repo_url`
2. push 时带上 `repo_url`，server 按 `repo_url` 判定是否同一项目
3. 不同用户的不同本地路径（`/Users/alice/code/ETerm` vs `/Users/bob/dev/ETerm`）指向同一 `repo_url` → server 归并为同一 project
4. ACL 以 server 端归并后的 project 为粒度

无 git remote 的项目（纯本地工程）fallback 到路径匹配 + 手动映射（config.json 中配 `path_aliases`）。

### 问题 4：sessions 表的纯本地字段

以下字段为增量采集的本地状态，不应同步：

| 字段 | 说明 |
|------|------|
| `file_mtime` | 文件修改时间戳 |
| `file_size` | 文件大小 |
| `file_offset` | 增量读取偏移 |
| `file_inode` | inode 编号 |
| `encoded_dir_name` | `~/.claude/projects/` 下的编码目录名 |

**同步字段白名单**：`session_id`, `project_path`, `cwd`, `model`, `channel`, `message_count`, `last_message_at`, `session_type`, `source`, `meta`, `created_at`, `updated_at`

### 问题 5：同步数据范围（算力分发模型）

**核心原则**：本地跑完所有计算（compact + embedding），把结果连同原始数据一起推到 server。Server 不跑 LLM、不跑 embedding，只存储 + 建索引 + 查询。

| DB 文件 | 表 | 是否同步 | 理由 |
|---------|----|---------|------|
| ai-cli-session.db | projects | ✅ | 项目元数据 |
| | sessions | ✅ | 会话元数据（去除本地字段） |
| | messages | ✅ | 核心数据（`raw` 字段可配置跳过，占 53% 体积） |
| | messages_fts | ❌ | 服务端从 content_text/content_full 自动重建 |
| | session_relations | ✅ | 父子会话关系 |
| | continuation_chains / nodes | ✅ | 续接链 |
| | talks | ✅ | L2/L3 摘要 |
| compact.db | observations (L1) | ✅ | 本地计算完成后推送 |
| | talk_summaries (L2) | ✅ | 本地计算完成后推送 |
| | session_summaries (L3) | ✅ | 本地计算完成后推送 |
| LanceDB | 向量 | ✅ | 本地 embedding 完成后推送（需统一 embedding 模型） |

> **约束**：所有客户端必须使用统一的 embedding 模型（如 `nomic-embed-text`），否则跨人向量搜索失效。FTS 不受影响。

### 问题 6：同步游标设计

`messages` 表准-append-only，有两类例外更新：

| 字段 | 变更方向 | 是否需要同步 |
|------|---------|------------|
| `vector_indexed` | 0→1→-1 | ❌ 各端独立索引 |
| `approval_status` | NULL→approved/rejected | ✅ 需要双向同步 |
| `approval_resolved_at` | NULL→timestamp | ✅ 需要双向同步 |

游标方案：每个 session 维护 `sync_cursor`（已同步的最后一条消息序号），服务端同样维护 per-device 游标，支持断点续传。

---

## 3. 整体架构

### 3.1 算力分发模型

所有计算（采集、compact 压缩、embedding 向量化）在本地完成，server 只做存储和查询。
本地不拉取同事数据，搜索采用 local-first + server fallback。

```
本地 A (你)                         Server (所有人)                    本地 B (同事)
┌─────────────────────────┐        ┌───────────────────────┐        ┌─────────────────────────┐
│ vimo-agent → 采集 L0    │        │                       │        │ vimo-agent → 采集 L0    │
│ CompactWorker → L1-L3   │─push─→│  SQLite (L0+L1-L3)    │←─push─│ CompactWorker → L1-L3   │
│ Embedding → 向量        │        │  LanceDB (向量)       │        │ Embedding → 向量        │
│                         │        │  FTS5 (自动重建)      │        │                         │
│ 本地搜索 (自己的数据)   │        │                       │        │ 本地搜索 (自己的数据)   │
│   │                     │        │  Auth + ACL           │        │   │                     │
│   └─ miss ─────────────────────→│  搜索 API (团队数据)  │←────────────┘                     │
│                         │        │                       │        │                         │
│ 不跑别人的 compact      │        │  不跑 LLM             │        │ 不跑别人的 compact      │
│ 不存别人的数据          │        │  不跑 Embedding       │        │ 不存别人的数据          │
└─────────────────────────┘        └───────────────────────┘        └─────────────────────────┘
```

### 3.2 数据量参考（基于当前 DB 统计）

| 指标 | 数值 |
|------|------|
| messages 总数 | ~150 万条 |
| DB 文件大小 | ~11 GB |
| `raw` 字段占比 | 5.5 GB (53%) |
| `content_full` 占比 | 1.9 GB (18%) |
| `content_text` 占比 | 154 MB (1.5%) |
| 去掉 `raw` 后 | ~5 GB |

> 同步时建议 `sync_raw: false`，可将传输和存储体积减半。

### 3.3 关键设计原则

1. **算力分发**：compact/embedding 在本地完成，server 零 LLM/GPU 开销
2. **本地零侵入**：sync 模块只读主 DB，同步状态写独立的 sync.db，主 DB 不可能被 sync 损坏
3. **本地唯一性**：每个客户端只存自己的数据，不膨胀
4. **Server 是 source of truth**：汇聚所有人的数据，提供跨人搜索
5. **本地保留原始数据**：敏感词过滤只在上传管道中发生
6. **搜索分层**：先查本地（快、完整）→ miss 了查 server（跨人、跨项目）
7. **Embedding 模型统一**：所有客户端必须使用相同模型，保证向量空间一致
8. **同一套 `memex-rs` 二进制**：通过 feature flag 区分 local 模式 vs server 模式

---

## 4. Schema 变更

### 4.1 projects 表（新增字段，主 DB）

```sql
ALTER TABLE projects ADD COLUMN repo_url TEXT;  -- git remote origin URL
ALTER TABLE projects ADD COLUMN team_id TEXT;   -- 服务端分组用（本地可为 NULL）
```

> 这是主 DB 唯一的 schema 变更。`repo_url` 由 vimo-agent 在采集时写入（`git remote get-url origin`），不涉及 sync 模块写入。

### 4.2 新增：sync.db（独立文件，仅本地）

**核心原则**：sync 模块对 ai-cli-session.db 和 compact.db 均以 `PRAGMA query_only=ON` 只读打开。所有同步状态写入独立的 `~/.vimo/db/sync.db`，与主 DB 完全隔离。sync.db 损坏可直接删除重建，最多重推一遍（server 端 uuid 去重保证幂等）。

```
~/.vimo/db/
├── ai-cli-session.db   ← vimo-agent 唯一写入者
├── compact.db          ← CompactWorker 唯一写入者
├── sync.db (新增)      ← sync 模块唯一写入者
└── lancedb/            ← indexer 唯一写入者
```

```sql
-- sync.db schema

-- 每个 session 的推送游标
CREATE TABLE IF NOT EXISTS sync_cursors (
    session_id   TEXT PRIMARY KEY,
    last_sequence INTEGER NOT NULL DEFAULT 0,
    last_push_at  INTEGER NOT NULL DEFAULT 0
);

-- compact 推送游标（按 session 粒度追踪 L1/L2/L3 推送进度）
CREATE TABLE IF NOT EXISTS compact_cursors (
    session_id   TEXT NOT NULL,
    level        TEXT NOT NULL,  -- 'L1' / 'L2' / 'L3'
    last_id      TEXT NOT NULL,  -- 最后推送的 observation/summary id
    last_push_at INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, level)
);

-- 向量推送游标
CREATE TABLE IF NOT EXISTS vector_cursors (
    session_id       TEXT PRIMARY KEY,
    last_message_id  INTEGER NOT NULL DEFAULT 0,
    last_push_at     INTEGER NOT NULL DEFAULT 0
);

-- 全局同步状态
CREATE TABLE IF NOT EXISTS sync_state (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
-- 存储: device_id, server_url, last_heartbeat, embedding_model 等
```

### 4.4 新增：users 表（仅服务端）

```sql
CREATE TABLE IF NOT EXISTS users (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    username     TEXT NOT NULL UNIQUE,
    api_key_hash TEXT NOT NULL,
    display_name TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
```

### 4.5 新增：project_access 表（仅服务端）

```sql
CREATE TABLE IF NOT EXISTS project_access (
    user_id    INTEGER NOT NULL REFERENCES users(id),
    project_id INTEGER NOT NULL REFERENCES projects(id),
    role       TEXT NOT NULL DEFAULT 'member',  -- admin / member / viewer
    created_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, project_id)
);
```

---

## 5. 同步协议设计

### 5.1 推送（本地 → 服务端）

本地计算完成后，增量推送 L0 原始数据 + L1-L3 compact 摘要 + 向量。

```
POST /api/sync/push
Authorization: Bearer <api_key>
Content-Type: application/json

{
  "device_id": "<uuid>",
  "batch": {
    "projects": [
      { "path": "/Users/alice/code/ETerm", "name": "ETerm", "repo_url": "git@github.com:vimo-ai/ETerm.git", "source": "claude" }
    ],
    "sessions": [
      { "session_id": "abc123", "project_path": "/Users/alice/code/ETerm", "cwd": "...", "model": "claude-opus-4", ... }
    ],
    "messages": [
      { "uuid": "msg-uuid-1", "session_id": "abc123", "content_text": "...", "role": 1, "timestamp": 1234567890, ... }
    ],
    "session_relations": [ ... ],
    "continuation_chains": [ ... ],
    "talks": [ ... ],
    "compact": {
      "observations": [ ... ],
      "talk_summaries": [ ... ],
      "session_summaries": [ ... ]
    },
    "vectors": [
      { "message_uuid": "msg-uuid-1", "embedding": [0.1, 0.2, ...], "chunk_index": 0 }
    ]
  },
  "cursors": {
    "abc123": { "last_sequence": 42, "last_timestamp": 1234567890 }
  }
}

响应:
{
  "accepted": 42,
  "skipped": 3,
  "server_cursors": {
    "abc123": { "last_sequence": 42 }
  }
}
```

### 5.2 搜索（本地 miss → 查 server）

本地不拉取同事的数据。搜索时先查本地，未命中或需要跨人搜索时调用 server 搜索 API。

```
GET /api/search?q=<keyword>&level=sessions&limit=20
Authorization: Bearer <api_key>

响应:
{
  "results": [
    {
      "session_id": "def456",
      "project": "ETerm",
      "user": "bob",
      "summary": "...",
      "score": 0.85,
      "highlights": ["...matched context..."]
    }
  ],
  "total": 5,
  "has_more": false
}
```

支持的搜索模式：
- **FTS**：`GET /api/search?q=<keyword>&mode=fts` — 关键词全文搜索
- **向量**：`GET /api/search?q=<text>&mode=vector` — 语义搜索
- **混合**：`GET /api/search?q=<text>&mode=hybrid` — RRF 融合排序（默认）

搜索结果按 ACL 过滤，只返回当前用户有权限的项目数据。

### 5.3 冲突解决策略

| 数据类型 | 冲突场景 | 解决策略 |
|---------|---------|---------|
| messages | 同 uuid 多次上传 | `ON CONFLICT(uuid) DO NOTHING`（append-only） |
| sessions | 元数据字段更新 | 以 `updated_at` 为准，last-write-wins |
| projects | 不同路径同 repo | 服务端按 `repo_url` 归并，保留所有 path 记录 |
| compact | 同 session 不同用户不会冲突 | 按 `(session_id, user_id)` 隔离 |
| vectors | 同 message_uuid 重复上传 | 按 `(message_uuid, chunk_index)` 去重 |
| approval_status | 多端同时审批 | 以 `approval_resolved_at` 最大值为准 |

### 5.4 推送策略与稳定性

#### 触发时机（事件驱动 + 兜底定时）

| 触发源 | 时机 | 推送内容 |
|--------|------|---------|
| vimo-agent collect 完成 | 新消息入库后 | L0 增量 messages |
| CompactWorker 完成 | L2/L3 生成后 | compact 增量摘要 |
| Indexer 完成 | embedding 完成后 | 向量增量 |
| 兜底定时器 | 每 5 分钟 | 检查所有 cursor，补推遗漏 |

#### 批次控制

- 按 session 分批推送，不跨 session 混批
- 每批上限：500 条 messages 或 2MB payload（先到先停）
- 一个 session 推完、server 确认后才更新 sync.db 中的 cursor

#### 断链与恢复

| 场景 | 处理方式 |
|------|---------|
| 网络闪断 | reqwest timeout 10s + 指数退避重试（最多 3 次，间隔 1s/4s/16s） |
| server 不可达 | 静默跳过，cursor 不动，下次定时兜底重推 |
| 长时间离线 | 上线后从 cursor 断点续传，积压按 session 逐批推 |
| push 部分成功 | server 返回 `accepted < sent` 时 cursor 不推进，整批下次重推 |
| sync.db 损坏 | 删除重建，cursor 归零，全量重推（server uuid 去重保证幂等） |

#### 数据一致性保证

1. sync 模块以 `PRAGMA query_only=ON` 打开主 DB → 不可能写坏主 DB
2. push 成功 → 更新 sync.db cursor → 原子操作（sync.db 事务）
3. server 端 `ON CONFLICT(uuid) DO NOTHING` → 重推不重复
4. cursor 只在 server 确认后推进 → 不会跳过数据

---

## 6. 敏感词过滤模块

### 6.1 设计原则

- **过滤点在同步管道，不在采集端**：本地 DB 始终保留原始数据
- **服务端收到的是已过滤版本**：服务端不存储原始敏感信息
- **使用 aho-corasick crate**：支持大规模多模式匹配，性能优秀

### 6.2 过滤字段

```
messages 表:
  - content_text   (FTS 文本)
  - content_full   (完整文本)
  - raw            (原始 JSONL，可配置是否同步)
  - tool_args      (工具参数)
```

### 6.3 过滤模式

| 模式 | 行为 | 适用场景 |
|------|------|---------|
| `redact` | 替换为 `***`（默认） | 需保留消息结构 |
| `block` | 跳过整条消息不上传 | 严格保密场景 |
| `tag` | 保留内容但标记 `flagged=true` | 审计场景 |

### 6.4 配置示例

```json
{
  "filter": {
    "enabled": true,
    "sensitive_words_file": "~/.vimo/memex/sensitive_words.txt",
    "mode": "redact",
    "remote_list_url": "https://internal/sensitive-words.txt"
  }
}
```

`sensitive_words.txt` 格式：每行一个词，支持 `#` 注释，支持正则（`/pattern/` 语法）。

---

## 7. 项目同步白名单

### 7.1 配置驱动

```json
{
  "sync": {
    "include_projects": ["/Users/*/code/*"],
    "exclude_projects": ["**/playground-*", "**/scratch", "**/tmp"]
  }
}
```

使用 `globset` crate 进行 glob 匹配。

### 7.2 执行位置

过滤在 sync 模块的推送前执行，不影响采集端和本地查询。逻辑：

```
include_projects 为空 → 全部项目参与同步
include_projects 非空 → 只有匹配的项目参与同步
exclude_projects 优先于 include_projects
```

---

## 8. 传输与存储安全

### 8.1 分阶段策略

| 阶段 | 传输加密 | 存储加密 | 说明 |
|------|---------|---------|------|
| Phase 1 | HTTPS/TLS | 明文 | Server 在自有 NAS 上，TLS 足够 |
| Phase 2 | TLS + AES-256-GCM | 明文 | 应用层加密双保险，防中间人嗅探 |

### 8.2 Phase 1：TLS

- NAS 配 TLS 证书（Let's Encrypt 或自签），HTTPS 接入
- memex-rs server-mode 启用 `rustls` 或反代（nginx/Caddy）termination
- 足够覆盖内网/VPN 场景

### 8.3 Phase 2：应用层 AES（后续升级）

- 客户端 push 前对 payload 做 AES-256-GCM 加密
- server 收到后解密，**明文写入 SQLite**（保证 FTS/向量搜索可用）
- 密钥管理：团队共享 symmetric key，配置在 `config.json` 的 `sync.encryption_key`
- AES 只保护传输管道，server 端存明文能搜索
- 协议预留：push payload 增加 `encryption: "none" | "aes-256-gcm"` 字段，Phase 1 固定 `"none"`

---

## 9. 新增模块结构

```
ai-cli-session-db/src/
  sync/
    mod.rs          -- 协议类型定义（SyncPayload, SyncBatch, SyncCursor）
    client.rs       -- 本地 → 服务端 push 逻辑（L0 + compact + 向量）
    server.rs       -- 服务端接收 + 写入 + 冲突解决
    cursor.rs       -- sync.db 游标管理（只读主 DB，读写 sync.db）
    filter.rs       -- 敏感词过滤（aho-corasick）+ 项目白名单（globset）
    normalize.rs    -- project path → repo_url 标准化
    db.rs           -- sync.db 连接 + schema 初始化

memex-rs/src/
  auth/
    mod.rs
    middleware.rs   -- axum Auth 中间件（API Key 验证）
    acl.rs          -- 项目级 ACL（user_id + project_id → role）
  server/
    mod.rs          -- server-mode 入口
    search.rs       -- 服务端搜索 API（FTS + 向量 + 混合，复用本地搜索逻辑）
    ingest.rs       -- 接收 push 数据，写入 SQLite + compact.db + LanceDB
```

### DB 读写权限矩阵

| 模块 | ai-cli-session.db | compact.db | sync.db | lancedb |
|------|-------------------|------------|---------|---------|
| vimo-agent | **读写** | - | - | - |
| CompactWorker | 读 | **读写** | - | - |
| Indexer | 读 | - | - | **读写** |
| sync 模块 | 只读 (`query_only`) | 只读 (`query_only`) | **读写** | 只读 |

---

## 10. 完整配置示例

```json
{
  "sync": {
    "enabled": true,
    "server": "https://memex.internal:10013",
    "api_key": "mk_xxxxxxxxxxxx",
    "interval_seconds": 300,
    "include_projects": ["/Users/*/code/*"],
    "exclude_projects": ["**/playground-*", "**/scratch"],
    "sync_raw": false,
    "sync_vectors": true,
    "embedding_model": "nomic-embed-text",
    "encryption": "none",
    "batch_size": 500,
    "compress": true
  },
  "filter": {
    "enabled": true,
    "sensitive_words_file": "~/.vimo/memex/sensitive_words.txt",
    "mode": "redact",
    "remote_list_url": null
  }
}
```

配置文件位置：`~/.vimo/memex/config.json`

---

## 11. 实施计划

### Phase 1：基础改造（预计 1 周）

- [ ] Schema 迁移：projects 加 `repo_url`（vimo-agent 写入，不涉及 sync 模块）
- [ ] `sync/db.rs`：sync.db 独立文件初始化（sync_cursors + compact_cursors + vector_cursors + sync_state）
- [ ] `normalize.rs`：`git remote get-url origin` 采集时写入 `repo_url`，失败时 fallback 到 path
- [ ] `filter.rs`：敏感词过滤（aho-corasick），支持 redact/block/tag 三种模式
- [ ] `filter.rs`：项目白名单逻辑（globset）
- [ ] 统一 embedding 模型配置项，写入 sync config

### Phase 2：服务端模式（预计 1 周）

- [ ] `memex-rs` feature flag `server-mode`，启用后加载 auth/acl/ingest 中间件
- [ ] `auth/middleware.rs`：API Key 验证（header `Authorization: Bearer <key>`）
- [ ] `users` + `project_access` 表服务端迁移
- [ ] `server/ingest.rs`：接收 push 数据，写入 L0 + compact + 向量
- [ ] `server/search.rs`：服务端搜索 API（FTS + 向量 + 混合），复用本地搜索模块

### Phase 3：同步协议（预计 1.5 周）

- [ ] `sync/mod.rs`：SyncBatch 协议类型（含 compact + vectors 字段）
- [ ] `sync/server.rs`：Push API — 接收 batch，repo_url 归并，uuid 去重，compact/向量入库
- [ ] `sync/cursor.rs`：per-session 游标管理，支持断点续传
- [ ] ACL 拦截搜索 API（按 project_id + user_id 过滤）

### Phase 4：客户端集成（预计 0.5 周）

- [ ] `sync/client.rs`：push 逻辑（L0 + compact + 向量），读取 sync_cursor，组装 SyncBatch
- [ ] 配置解析：server URL、api_key、interval、include/exclude_projects、embedding_model
- [ ] 后台同步 Worker（基于 tokio interval，周期性 push）
- [ ] 本地搜索 miss → server fallback 逻辑
- [ ] 手动触发同步 API（`POST /api/sync/trigger`）

### Phase 5：后续优化（可选）

- [ ] 同步状态 API（`GET /api/sync/status`），供 ETerm / iOS 显示同步进度
- [ ] 搜索结果合并 UI：本地结果 + server 结果分层展示
- [ ] Web 管理界面：用户管理、项目权限配置
- [ ] 限流 + 批次大小控制（防止大规模首次同步打爆服务端）
- [ ] 大 payload gzip 压缩
- [ ] 敏感词列表远程 URL + 本地缓存

---

## 12. 风险评估

| 风险 | 影响 | 缓解方案 |
|------|------|---------|
| project path 无 git remote（纯本地工程） | 中 | fallback 到 path 匹配；支持手动在 config.json 中配置 path→name 映射 |
| `messages.raw` 字段过大，导致 batch payload 膨胀 | 高 | 配置项 `sync_raw: false` 跳过 raw 字段（省 53% 体积）；启用 gzip 压缩 |
| embedding 模型不一致导致跨人向量搜索失效 | 高 | 配置项 `embedding_model` 强制统一；server 端校验模型名一致性 |
| 服务端 FTS 重建在大数据量下耗时长 | 中 | 异步后台重建，不阻塞 push 写入 |
| SQLite `SQLITE_BUSY` 超时（多人同时 push） | 低 | 各人 session 不重叠，WAL + busy_timeout 串行处理足够 |
| 首次全量同步数据量大（存量几十万条消息 + compact + 向量） | 高 | 分 session 分批推送；服务端幂等（uuid dedup）；支持断点续传 |
| 向量数据传输体积（高维 float 数组） | 中 | 量化压缩（float32→float16）；按需同步（仅同步白名单项目的向量） |
| 敏感词列表维护成本高 | 低 | 支持远程 URL 拉取列表 + 本地覆盖；列表版本化管理 |

---

## 13. 部署方案

### Server 端：群晖 NAS + Docker

```yaml
# docker-compose.yml
version: "3"
services:
  memex-server:
    image: vimo/memex-rs:latest
    command: ["memex", "--server-mode"]
    ports:
      - "10013:10013"
    volumes:
      - /volume1/docker/memex/db:/data/db
      - /volume1/docker/memex/config.json:/etc/memex/config.json:ro
    environment:
      - MEMEX_DB_PATH=/data/db
      - MEMEX_SERVER_MODE=true
    restart: unless-stopped
```

存储路径：
- `/volume1/docker/memex/db/ai-cli-session.db` — 全员汇聚的 L0 数据
- `/volume1/docker/memex/db/compact.db` — 全员的 compact 摘要
- `/volume1/docker/memex/db/lancedb/` — 全员的向量索引

通过已有 DNS 解析域名，同事直接配 `server: "https://memex.your-domain.com:10013"`。

### Client 端：本地 memex-rs

无额外部署，现有 memex-rs 加载 sync 配置后自动启动后台 push worker。

---

## 14. 参考

- [ai-cli-session-db 架构](./session-db-agent.md)
- [多源会话数据结构](./session-chain-linking.md)
- [Agent V2 读写分离](./agent-v2-rw-separation.md)
- aho-corasick crate: https://docs.rs/aho-corasick
- globset crate: https://docs.rs/globset
