# Memex 团队分发方案

> 2026-04-25 · 作战文档 · 状态：设计对齐

## 目标

将 memex 作为商业化产品分发给团队成员，实现：
- 强制上报会话数据到固定服务器，不可配置 filter
- 按人、按项目、按账号维度统计 Claude 用量
- 检测工作机之外的账号消耗（防止账号外用）

## 已有基建

### 完整的双向 sync 系统（已跑通）

**客户端（ai-cli-session-db/src/sync/）：**
- `SyncClient` — 从本地 DB 只读组装 SyncBatch，push 到 server
- `SyncWorker` — 事件驱动（文件变更 → `trigger_session`）+ 定时全量兜底
- `ProjectFilter` — include/exclude glob 白名单
- `SensitiveWordFilter` — Aho-Corasick 敏感词（redact/block/tag）
- `SyncDb` — 独立 sync.db 记录游标，不污染主库
- `SyncStream` — WebSocket 持久连接，心跳保活

**服务端（memex-rs/src/server/ + bin/memex-server.rs）：**
- `POST /api/sync/push` — HTTP 批量接收
- `WebSocket /api/sync/stream` — 持久连接接收
- `POST /api/auth/register` — master_key 自注册，幂等
- `IngestState` — 复用 ai-cli-session-db schema + `pushed_by` 字段
- `sync_devices` 表 — 设备心跳追踪
- TLS 支持（证书配置）
- FTS 搜索 API

**SyncBatch 结构（已定义）：**
- projects / sessions / messages / talks / session_relations / continuation_chains / chain_nodes

### ClaudeMonitorKit（ETerm 插件，已有）

- `WeeklyUsageTracker` — 调 Anthropic OAuth API 采集 utilization 百分比
- `UsageHistoryStore` — 持久化到 JSON，周期检测
- `SprintPredictor` — 加权预测配额消耗节奏

### Statusline Token 模块（vlaude-cli，已有）

- `tokens.ts` — 从 JSONL 提取 token 四件套，算加权成本和 ×N 倍率
- `context.ts` — 算 context 占用百分比
- 加权公式：`cost = cache_read×1 + cache_create×16 + input×10 + output×50`

## 需要做的改动

### Phase 1：锁死配置 + Token 字段

**1.1 锁死 SyncConfig（编译时 / feature flag）**

团队版客户端：
- `enabled` = true，不可关闭
- `server` = 固定地址，不从 config.json 读
- `include_projects` / `exclude_projects` = 空（全量上报）
- `FilterConfig.enabled` = false（禁用敏感词过滤）
- `sync_raw` = false（不需要传 raw，结构化字段够用）

实现方式待定：`--features team` 编译开关 or 配置文件签名校验

**1.2 Token 四件套加到采集链路**

当前断点：
- `ai-cli-session-collector` Claude adapter 未提取 `message.usage`（OpenCode/Gemini 的有）
- `messages` 表无 token 字段
- `SyncMessage` 无 token 字段

需要加的字段（per message，来自 JSONL `message.usage`）：
```
input_tokens           INTEGER
output_tokens          INTEGER
cache_read_input_tokens    INTEGER
cache_creation_input_tokens    INTEGER
```

改动链路：
1. `ai-cli-session-collector` Claude adapter — 提取 usage
2. `ai-cli-session-db` schema — messages 表加列
3. `ai-cli-session-db` writer — 写入 token 字段
4. `SyncMessage` — 加 token 字段
5. `memex-server` ingest — 接收并存储

### Phase 2：Utilization + 账号身份上报

**数据源：Anthropic OAuth API**

```
GET /api/oauth/profile → account.uuid, email, rate_limit_tier
GET /api/oauth/usage   → seven_day.utilization, five_hour.utilization
```

凭据来源：Keychain `Claude Code-credentials` → `claudeAiOauth.accessToken`

Profile 返回示例：
```json
{
  "account": {
    "uuid": "ef223c81-...",
    "email": "kt.shaoche@gmail.com",
    "has_claude_max": true
  },
  "organization": {
    "uuid": "f3cb01e1-...",
    "rate_limit_tier": "default_claude_max_20x"
  }
}
```

**上报设计：**
- 客户端定时采集 utilization + account info，随 sync 推到服务端
- 服务端按 `account.uuid` + 时间戳去重（多台机器同一个号只记一份）
- 需要新建 `utilization_samples` 表：
  ```
  account_uuid    TEXT
  account_email   TEXT
  rate_limit_tier TEXT
  pushed_by       TEXT       -- 哪台机器上报的
  seven_day_util  REAL
  five_hour_util  REAL
  sampled_at      INTEGER
  ```

### Phase 3：服务端分析

- 按 `pushed_by`（人）× `project_path`（项目）汇总 token 消耗
- 按 `account_uuid`（账号）汇总 utilization 变化曲线
- **差值检测**：账号 utilization 对应的总消耗 vs 该账号下所有工作机上报的 token 总和，差值 = 外部消耗

## 不做 / 不追踪

- ❌ **1% utilization 精确标定**：Anthropic 算法黑盒且持续变化，多号切换会污染标定数据。Token 计数 + utilization 分别记录就够用，不做换算系数拟合
- ❌ **Raw JSONL 上传**：现有结构化 sync 已经覆盖所有需要的字段，raw 是冗余数据纯浪费带宽
- ❌ **配额分配系统**：先做监控（看谁用了多少），不做配额管控（限制谁能用多少）
- ❌ **Dashboard UI**：先出 API，前端后做
- ❌ **statusline ×N 倍率的服务端复现**：这是个人效率工具，团队场景看 token 绝对值更直接
- ❌ **本地 memex 功能裁剪**：保留本地搜索/compact/MCP 等能力，对使用者本人有价值，不砍

## 数据流全景

```
团队成员工作机
├── Claude Code → JSONL → vimo-agent → 本地 DB
│                                         │
│   ┌─────────────────────────────────────┘
│   │
│   ├── SyncWorker（事件驱动 + 定时兜底）
│   │   ├── SyncBatch（projects/sessions/messages + token 四件套）
│   │   └── pushed_by = 注册用户名
│   │
│   ├── Utilization 采集（定时）
│   │   ├── GET /api/oauth/profile → account_uuid, email
│   │   └── GET /api/oauth/usage  → utilization %
│   │
│   └──→ 中央 memex-server（WebSocket stream）
│
本地 memex（搜索/compact/MCP，可选，独立运行）


中央 memex-server
├── 收 SyncBatch → server.db（复用 schema + pushed_by）
├── 收 utilization → utilization_samples（按 account_uuid 去重）
├── 搜索 API（FTS）
└── 统计 API（按人/项目/账号维度）
    ├── token 消耗：SUM(input/output/cache) GROUP BY pushed_by, project_path
    ├── utilization 曲线：按 account_uuid 时间序列
    └── 差值检测：utilization 总消耗 - 工作机 token 总和 = 外部消耗
```

## 优先级

1. **Phase 1** — 锁配置 + token 字段（改动集中，风险低）
2. **Phase 2** — utilization + 账号上报（新增模块，但逻辑简单）
3. **Phase 3** — 服务端统计 API（依赖前两步数据就位）

## 多组多号扩展

当前假设：一个号全公司共用。后续拆组拆号（测试组 A 号、开发组 B 号）时：
- Token 数据按 `pushed_by` 归人，不受拆号影响
- Utilization 数据按 `account_uuid` 归号，自动适配
- 服务端只需加 account → group 的映射配置
