# L4 Knowledge Pipeline — Rust 移植作战文档

> 2026-04-28，基于 Python 脚本验证结果 + memex-rs 架构分析

## 目标

将 L4 知识结晶 pipeline 从外挂 Python 脚本移入 memex-rs 原生模块，作为 compact 体系的自然延伸。

**不做**：L5 域分类（保留 schema 口子）、L6 跨项目聚合、自动触发。

## 现状

### 已验证的 Python pipeline

| 脚本 | 功能 | LLM 调用 |
|------|------|---------|
| `l4-pipeline.py` | Session → knowledge nodes（2-pass 提取） | Chat × 2/session |
| `l4-cluster.py` | Embedding 聚类 + LLM 命名 | Embedding × N + Chat × batch |
| `l4-merge.py` | LLM 判断 cluster 合并 | Chat × chunk |
| `l4-evolve.py` | 跨时间演化关系分析 | Chat × cluster |
| `l4-import.py` | 写入 DB | 无 |
| `l4-incremental.py` | 上述全部合一（增量） | 全部 |

全量 pipeline 5 步串行，增量 pipeline 1 个脚本搞定。**正式架构只需增量模式**，全量 = 增量跑 N 次。

### DB 现状（2026-04-28）

| 项目 | clusters | nodes | 覆盖率 |
|------|----------|-------|--------|
| keeta | 4523 | 9448 | 96% |
| 工厂 | 843 | 1289 | 100% |
| ETerm | 262 | 488 | — |
| 其他 | 327 | 514 | — |
| **总计** | **5955** | **11739** | — |

### memex-rs 已有能力（直接复用）

| 能力 | 位置 | L4 用法 |
|------|------|--------|
| `ChatProvider` trait | `llm/chat.rs` | 提取 + 演化分析 |
| `EmbeddingProvider` trait | `llm/embedding.rs` | node→cluster 匹配 |
| `OllamaProvider` | `llm/providers/ollama.rs` | 本地 GPU |
| `OpenAIProvider` | `llm/providers/openai.rs` | 远端 API |
| `LlmClientCore` | `llm/core.rs` | HTTP + auth |
| `DbReader` | `db_reader.rs` | 读 session DB 的 messages |
| `CompactDB` watermark 模式 | `compact/db.rs` | progress 参考 |
| `embed_batch` (10 并发) | ollama provider | 批量 embedding |

**不需要新的 provider 或 HTTP 层。**

---

## 架构

### 模块位置

```
src/
├── compact/        # L0→L3（已有，自动触发，轻量模型）
├── knowledge/      # L4+（新建，手动触发，强模型）
│   ├── mod.rs
│   ├── config.rs       # KnowledgeConfig
│   ├── prompt.rs       # 提取/演化 prompt 模板
│   ├── extractor.rs    # session → knowledge nodes
│   ├── matcher.rs      # embedding match + cluster
│   ├── store.rs        # knowledge_* 表读写 + progress
│   └── service.rs      # 编排入口
├── llm/            # AI 抽象层（共享）
├── mcp/            # MCP 工具（改造 search_history）
└── ...
```

### 为什么不放 compact/ 内

| | compact | knowledge |
|---|---------|-----------|
| 触发 | 自动（session idle 5min） | 手动（CLI / MCP tool） |
| 模型 | 轻量（qwen3:0.6b） | 强（qwen3:14b / Claude API） |
| 成本 | 低（每 session 几秒） | 高（每 session 15-30s） |
| 写入 | compact.db | session DB (knowledge_* 表) |
| 生命周期 | 跟随 session | 跟随主题演化 |

两者共享 `llm/` 和 `db_reader`，但职责和运行模式完全不同。

### Config 设计

`~/.vimo/memex/config.json`:

```json
{
  "compact": { "enabled": true, ... },
  "knowledge": {
    "enabled": false,
    "chat_model": "qwen3:14b",
    "embedding_model": "nomic-embed-text",
    "match_threshold": 0.82,
    "cluster_threshold": 0.76,
    "pipeline_version": "v1"
  }
}
```

- provider endpoint 复用全局 `ollama.api`（或 OpenAI 配置），model name 独立
- `enabled` 默认 false — 需要用户显式开启
- 阈值从 Python 脚本继承，已在 keeta/工厂数据上验证

### Provider 选择逻辑

```
用户配置了 knowledge.chat_model?
  ├─ 是 → 用全局 ollama/openai endpoint + knowledge.chat_model
  └─ 否 → 用全局 chat_model（可能不够强，warn 提示）

Embedding 同理，复用全局 embedding provider 实例
```

**同构**：不区分本地/API/远端模式，ChatProvider trait 统一处理。

---

## 实施步骤

### Phase 1: 骨架 + Store

**目标**：模块能编译，schema 能建，progress 能读写。

1. **新建 `src/knowledge/` 模块结构**
   - `mod.rs` — 模块声明 + pub use
   - `config.rs` — `KnowledgeConfig` struct（serde 反序列化）
   - `store.rs` — knowledge 表 CRUD + progress tracking

2. **注册到 `lib.rs` 和 `config.rs`**
   - `lib.rs` 加 `pub mod knowledge;`
   - `config.rs` 的 `FileConfig` 加 `knowledge: KnowledgeConfig`
   - `Config` struct 加 `pub knowledge: KnowledgeConfig`

3. **Store 实现**
   - `ensure_schema()` — 从 `l4-schema.sql` 内容内嵌，CREATE IF NOT EXISTS
   - `get_progress(session_id)` / `update_progress(session_id, offset)` — 新建 `knowledge_progress` 表
   - `get_unprocessed_sessions(project_ids, limit)` — 查 sessions 表排除已处理的
   - `insert_nodes(nodes)` / `insert_cluster(cluster)` — 写入 knowledge_nodes/clusters
   - `load_clusters(project_ids)` — 读已有 clusters 用于匹配
   - `update_cluster_stats(cluster_id)` — 更新 node_count/day_count 等
   - `upsert_relations(relations)` — 写入 knowledge_relations

4. **Progress 表 schema**
   ```sql
   CREATE TABLE IF NOT EXISTS knowledge_progress (
       session_id TEXT PRIMARY KEY,
       status     TEXT NOT NULL DEFAULT 'pending',  -- pending/extracted/failed
       node_count INTEGER DEFAULT 0,
       error      TEXT,
       processed_at INTEGER,
       pipeline_version TEXT
   );
   ```

**验证**：单元测试 — 建表、写入、查询 round-trip。

### Phase 2: Extractor（Stage 1）

**目标**：给一个 session_id，产出 knowledge nodes。

1. **`prompt.rs`** — 移植 Python 的 PASS1_PROMPT + PASS2_PROMPT + EVOLVE_PROMPT
   - 模板字符串，`format!()` 填入 conversation digest 和 project desc
   - `try_parse_json()` — strip markdown fence + 括号匹配 + trailing comma 修复

2. **`extractor.rs`**
   - `extract_conversation_digest(db, session_id)` — 从 messages 表构建纯对话内容
     - **输入源**：L0 原始消息，不使用任何 L1-L3 产出（避免低质量摘要污染）
     - **按 subtype 过滤**，不做截断：
       - ✅ 保留：`human_input`（用户意图）+ `assistant` 回复（结论来源）
       - ❌ 跳过：`tool_result`（占 79%，过程噪音）、`tool_call` 参数、`command`、`system_reminder`
     - subtype 判断：raw 字段含 `"tool_result"` → 跳过；含 `<command-name>` → 跳过；含 `<system-reminder>` → 跳过
     - 过滤后自然控制长度（去掉 tool_result 后通常缩减 70%+），不需要硬截断
     - 如果配置了 `redact_before_llm`，digest 过一遍 `SensitiveWordFilter`
   - `extract_nodes(chat_provider, digest, project_desc)` → `Vec<KnowledgeNode>`
     - Pass 1: `chat_with_system(SYSTEM, PASS1_PROMPT)` → 自由分析
     - Pass 2: `chat_with_system(SYSTEM, PASS2_PROMPT)` → JSON 结构
     - JSON 解析失败时 retry 一次
   - `KnowledgeNode` struct: `{ id, session_id, project_id, day, topic, conclusion, node_type, confidence }`
   - Node ID 生成: `sha256(session_id|topic|conclusion[:50])[:16]`

**设计决策：为什么 L4 直接读 L0**

```
L0 原始消息
├── compact 链路（0.6B，有损加速）: L0 → L1 → L2 → L3
└── knowledge 链路（强模型，高质量）: L0 → L4
```

两条并行 pipeline，不是串行。L4 从 L0 读不是"跳层"。
从 L2/L3 读会继承 0.6B 的信息损失，且前序低质量摘要会 anchor 强模型的判断（污染）。

**验证**：集成测试 — 用一个已知 session 跑提取，对比 Python 产出。

### Phase 3: Matcher（增量合入）

**目标**：新 nodes 匹配到已有 clusters 或自建新 cluster。

1. **`matcher.rs`**
   - `match_nodes_to_clusters(embed_provider, new_nodes, existing_clusters, threshold)` → `(matched, unmatched)`
     - 批量 embed 新 nodes（`embed_batch`）
     - 批量 embed 已有 cluster centroids
     - Cosine similarity matrix（手写或用 `ndarray`）
     - 超过 `match_threshold` 的归入已有 cluster
   - `cluster_unmatched(embed_provider, unmatched_nodes, threshold)` → `(multi_clusters, singletons)`
     - 未匹配节点之间互聚类
   - `name_clusters(chat_provider, clusters)` → `Vec<String>`
     - 批量 LLM 命名（50 个一批）
   - `evolve_cluster(chat_provider, cluster, nodes)` → `Evolution`
     - 跑 EVOLVE_PROMPT，产出 current_understanding + relations
     - 最少 3 nodes + 2 days 才触发

2. **Cosine similarity**
   - 小规模（<100×100）：手写 `Vec<f32>` 点积
   - 大规模：用 `ndarray` 矩阵乘（Cargo.toml 加 optional dependency）

**验证**：用 keeta 的已有 nodes 做匹配测试，对比 Python 的 match 结果。

### Phase 4: Service 编排

**目标**：串起完整流程，对外暴露单一入口。

1. **`service.rs`** — `KnowledgeService`
   ```rust
   pub struct KnowledgeService {
       store: KnowledgeStore,
       chat: Arc<dyn ChatProvider>,
       embed: Arc<dyn EmbeddingProvider>,
       config: KnowledgeConfig,
   }
   
   impl KnowledgeService {
       /// 处理指定 sessions（核心方法）
       pub async fn process_sessions(&self, session_ids: Vec<String>) -> ProcessResult;
       
       /// 处理指定项目的所有未处理 sessions
       pub async fn process_project(&self, project_id: i64, limit: Option<usize>) -> ProcessResult;
       
       /// 查询 session 关联的 knowledge（给 MCP 用）
       pub fn get_knowledge_for_sessions(&self, session_ids: &[String]) -> Vec<SessionKnowledge>;
   }
   ```

2. **`process_sessions` 流程**（对应 `l4-incremental.py`）：
   ```
   for session in sessions:
       1. extract_conversation_digest(session)
       2. extract_nodes(digest)           → new_nodes
       3. update_progress(session, extracted)
   
   4. load_existing_clusters(project_ids)
   5. match_nodes_to_clusters(new_nodes, clusters)
   6. cluster_unmatched(unmatched)
   7. name_clusters(new_clusters)
   8. write_to_db(matched + new_clusters + singletons)
   9. evolve_affected_clusters(updated + new)
   ```

3. **中断恢复**：每个 session 提取完立即写 progress，匹配/入库阶段失败可从"已提取未入库"的 nodes 恢复。

**验证**：端到端测试 — 选 10 个 session 跑完整流程，检查 DB 结果。

### Phase 5: 触发入口

**目标**：用户能通过 MCP 和 CLI 触发 L4 pipeline。

1. **MCP tool**（`mcp/mod.rs`）
   - 新增 `extract_knowledge` tool:
     ```json
     {
       "name": "extract_knowledge",
       "description": "Extract L4 knowledge nodes from sessions",
       "inputSchema": {
         "project": "project path or glob pattern",
         "limit": "max sessions to process (default 10)",
         "dry_run": "preview only, don't write DB"
       }
     }
     ```
   - 返回：processed count + new nodes + matched/new clusters

2. **MCP 改造 `search_history`**（核心消费端）
   - L3 session 结果返回时，附带关联的 L4 knowledge:
     ```json
     {
       "session": "uuid",
       "summary": "...",
       "knowledge": [
         {
           "topic": "Session Key Derivation",
           "conclusion": "session_key = BASE_KEY ⊕ (counter ^ 0xa6)",
           "confidence": 0.92,
           "cluster": "mtgsig Key Management",
           "cluster_size": 12
         }
       ]
     }
     ```
   - 实现：拿到 session_ids 后，`SELECT kn.topic, kn.conclusion, kn.confidence, kc.canonical_topic, kc.node_count FROM knowledge_nodes kn JOIN knowledge_clusters kc ON kn.cluster_id = kc.id WHERE kn.session_id IN (?)`

3. **CLI 参数**（`main.rs`）
   - `--extract-knowledge --project <path> [--limit N] [--dry-run]`
   - 调用 `KnowledgeService::process_project()`

### Phase 6: Provider 初始化

**目标**：main.rs 中根据配置创建 L4 专用 provider 实例。

在 `main.rs` 的 provider 初始化段（现有 compact provider 旁边）：

```rust
// Compact provider（轻量模型）
let compact_provider = OllamaProvider::new(&config.ollama_api, &config.embedding_model, &config.chat_model);

// Knowledge provider（强模型，独立实例）
let knowledge_provider = if config.knowledge.enabled {
    Some(OllamaProvider::new(
        &config.ollama_api,
        &config.knowledge.embedding_model,
        &config.knowledge.chat_model,
    ))
} else {
    None
};
```

如果用户配了 OpenAI：
```rust
let knowledge_chat = OpenAIProvider::new(&openai_base, &openai_key, &config.knowledge.chat_model);
// embedding 仍走本地 ollama（OpenAI embedding 贵且不一定好）
let knowledge_embed = OllamaProvider::new(&config.ollama_api, &config.knowledge.embedding_model, "unused");
```

---

## 关键设计决策

### 1. 写到哪个 DB

**选择：session DB (`ai-cli-session.db`)**

- knowledge_nodes 有 FK 到 sessions 和 projects 表，放同一个 DB 避免跨库 JOIN
- MCP search_history 查 compact.db 做搜索 → 拿到 session_id → 再查 session DB 拿 knowledge，只多一次查询
- 现有 Python 脚本已经写在 session DB 里，数据不用迁移

### 2. 全量 vs 增量

**选择：只实现增量模式**

- `knowledge_progress` 表记录每个 session 的处理状态
- 全量 = 对所有 unprocessed sessions 跑增量
- 支持 `--limit N` 分批跑，避免一次性成本过高

### 3. L5 预留

- `knowledge_clusters.l5_domain` 字段已存在（TEXT, nullable）
- 将来加 `src/knowledge/l5/` 子模块，不影响 L4 结构
- 暂不建 knowledge_domains 表，当前 l5_domain 直接存字符串够用

### 4. 敏感内容

**复用 sync 的 `SensitiveWordFilter`**（`ai-cli-session-db/src/sync/filter.rs`）

sync push 管道已有完整的敏感词过滤：
- Aho-Corasick 自动机，支持 Redact（替换 `***`）/ Block（丢弃）/ Tag 三种模式
- 敏感词从外部文件加载（`sensitive_words_file` 配置项）
- 已在 sync push 中对 content_text / content_full / raw / tool_args 做 redact

L4 的做法：`extractor.rs` 构建 conversation digest 后、发给 LLM 前，用同一个 filter 实例过一遍 redact。跟 sync push 共享同一份敏感词文件，行为一致。

config 里加开关：
```json
{
  "knowledge": {
    "redact_before_llm": true
  }
}
```

注意：compact L3 目前没有做 redact（直接发原始消息给 LLM），后续应该统一补上，复用同一个 filter。

---

## 文件改动清单

| 文件 | 改动 |
|------|------|
| `src/knowledge/mod.rs` | **新建** |
| `src/knowledge/config.rs` | **新建** |
| `src/knowledge/prompt.rs` | **新建** |
| `src/knowledge/extractor.rs` | **新建** |
| `src/knowledge/matcher.rs` | **新建** |
| `src/knowledge/store.rs` | **新建** |
| `src/knowledge/service.rs` | **新建** |
| `src/lib.rs` | 加 `pub mod knowledge;` |
| `src/config.rs` | FileConfig 加 knowledge 字段，Config 加 knowledge 字段 |
| `src/mcp/mod.rs` | 加 `extract_knowledge` tool + search_history 带 knowledge |
| `src/main.rs` | knowledge provider 初始化 + CLI 参数 + AppState 扩展 |
| `Cargo.toml` | 可选: `ndarray` (大规模 cosine similarity) |

---

## 执行顺序

```
Phase 1  骨架 + Store        → 编译通过 + 表能建
Phase 2  Extractor           → 单 session 能提取
Phase 3  Matcher             → 能匹配 + 聚类
Phase 4  Service             → 端到端跑通
Phase 5  触发入口            → MCP + CLI 可用
Phase 6  Provider 初始化     → 完整集成到 main.rs
```

Phase 1-3 可以独立测试，Phase 4 串起来，Phase 5-6 接入系统。

---

## 风险与注意

1. **session DB 并发** — knowledge 写入和 vimo-agent 写入可能冲突，需要 WAL 模式 + busy_timeout（session DB 已经是 WAL）
2. **大规模 embedding** — keeta 4500+ clusters 的 centroid embedding 要在内存里做矩阵运算，注意内存占用
3. **LLM 调用失败** — 网络超时、模型返回垃圾 JSON 等，每步都要有 retry + graceful fallback
4. **Python 脚本保留** — Rust 移植完成前，Python 脚本作为参考和 fallback 保留在 `scripts/`
