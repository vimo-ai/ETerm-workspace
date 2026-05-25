# Memex Session Intelligence 设计文档

> 2026-04-25 初稿，基于讨论整理，待逐步收敛

---

## 一、问题背景

memex 已经有完整的数据采集链路：本地 Claude Code 会话 → compact（L0-L3）→ sync 推送到中央 server。jobin 等团队成员的数据已经在持续同步进来。

但目前数据只是"躺着" — 没有被利用的手段。核心诉求：

1. **避免团队重复工作** — 我在做的事情，可能别人已经做过了
2. **搜索结果降噪** — 过时的、错误的结论不应该排在前面
3. **知识沉淀** — 有价值的经验应该被识别和保留

## 二、架构：逐级结晶

### 设计哲学

不引入新范式（记忆宫殿、大脑模型、知识图谱等），而是**沿着 compact 体系自然延伸**。

调研了 2026 年主流记忆架构（MemPalace 空间隐喻、CLS 互补学习、做梦巩固、A-MEM Zettelkasten、MAGMA 多图、MemEvolve 元演化、Engram 印迹、HiMem 层级记忆等），结论是这个领域**没有成熟的范式** — 每个都是某个隐喻硬套上去的。memex 的 L0-L3 compact 本身就是一个自底向上长出来的架构，最优雅的做法是继续延伸它。

核心隐喻：**逐级结晶**。从液态对话到固态知识，每一层滤掉一层噪音。

### Compact 全景

```
L0  原始消息                    ← 已有
L1  消息级压缩                  ← 已有
L2  对话轮级摘要（talks）        ← 已有
L3  会话级摘要（session）        ← 已有
─── 以下为新增 ──────────────
L4  主题级知识（跨 session）     ← 本文核心
L5  项目级技术图谱（跨主题）     ← 最终产物
```

| 层级 | 输入 | 输出 | 粒度 | 生命周期 |
|------|------|------|------|---------|
| L0 | JSONL 原文 | 原始消息 | 消息 | 永久（归档） |
| L1 | L0 | 压缩消息 | 消息 | 随 L0 |
| L2 | L0 | 每轮对话摘要 | prompt-response | 随 session |
| L3 | L2 | 会话摘要 | session | 随 session |
| **L4** | L3 + chain | **主题知识节点** | **跨 session** | **随主题演化** |
| **L5** | L4 | **项目技术图谱** | **项目** | **随项目演化** |

### L4：主题级知识

#### Session Graph：L4 的输入单元

L4 的输入不是单条 session，而是一个 **session graph** — 由多种关系组合而成的完整工作上下文。

##### `session_refs` 统一引用表

所有 session 间关系存储在一张表中，`ref_type` 区分语义：

```sql
CREATE TABLE session_refs (
    source_session_id  TEXT NOT NULL,
    target_session_id  TEXT NOT NULL,
    ref_type           TEXT NOT NULL,
    confidence         REAL NOT NULL DEFAULT 1.0,
    context            TEXT,
    source_message_uuid TEXT,
    detected_at        INTEGER NOT NULL,
    PRIMARY KEY (source_session_id, target_session_id, ref_type)
);
```

| ref_type | 来源 | confidence | 含义 |
|----------|------|-----------|------|
| `continuation` | `[continuation_from: xxx]` 标记 | 1.0 | `/continue` 产生的顺序延续 |
| `orchestrate` | `session_type='subagent'` 推断 | 1.0 | `/orchestrate` 产生的并行分支 |
| `text_uuid` | 消息中出现完整 UUID | 0.95 | 用户在对话中引用另一个 session |
| `text_prefix` | 消息中出现 8 位前缀 | 0.8 | 用户用短 hash 引用 session |
| `text_agent_ref` | 消息中出现 `agent-xxx` | 0.9 | 用户引用 subagent session |

##### 为什么一张表

- 核心价值是**图查询**（"跟这个 session 相关的所有东西"），一张表一个 recursive CTE 搞定，不用 UNION 多表
- `WHERE ref_type = 'continuation'` 过滤效果等同于独立的 `continuation_chain_nodes` 表
- `confidence` 列分离结构化数据（≥1.0）和行为数据（<1.0），L4 按需卡阈值

##### 三类关系

```
session-A (root)
├── /continue → session-B          (continuation, conf=1.0)
│   ├── /orchestrate → agent-B1    (orchestrate, conf=1.0)
│   └── /orchestrate → agent-B2    (orchestrate, conf=1.0)
├── /continue → session-C          (continuation, conf=1.0)
│   └── /continue → session-D      (continuation, conf=1.0)
└── ← referenced by session-X      (text_prefix, conf=0.8)
     "在 a1b2c3d4 那个 session 里做过"
```

- **continuation**（纵向）：线性链，同一任务的多轮推进
- **orchestrate**（横向）：父→子树，任务并行分支
- **text reference**（跨域）：人在对话中引用其他 session，反映认知关联。类比论文引文网络——被引次数高的 session 是 "知识 hub"，引用关系链是推理路径

##### Graph 构建

从任意 session 出发：

```sql
-- 上溯到 chain root
WITH RECURSIVE ancestors(sid, depth) AS (
    VALUES(:session_id, 0)
    UNION ALL
    SELECT r.source_session_id, a.depth - 1
    FROM ancestors a
    JOIN session_refs r ON r.target_session_id = a.sid
        AND r.ref_type = 'continuation'
    WHERE a.depth > -50
)
SELECT sid FROM ancestors ORDER BY depth;

-- 下探所有后代（chain + orchestrate）
WITH RECURSIVE descendants(sid, depth) AS (
    VALUES(:root_id, 0)
    UNION ALL
    SELECT r.target_session_id, d.depth + 1
    FROM descendants d
    JOIN session_refs r ON r.source_session_id = d.sid
        AND r.ref_type IN ('continuation', 'orchestrate')
    WHERE d.depth < 50
)
SELECT sid FROM descendants;
```

L4 构建 session graph 时：
1. `ref_type IN ('continuation', 'orchestrate')` → 结构化图（确定性关系）
2. `ref_type LIKE 'text_%'` → 行为关联（用于 pre-grouping 和 hub 检测）

##### 扫描与增量更新

扫描工具：`scripts/session-refs.py`

```bash
python3 scripts/session-refs.py                 # 全量扫描，写入 DB
python3 scripts/session-refs.py --incremental   # 只扫新消息
python3 scripts/session-refs.py --stats         # 查看图谱统计
```

增量模式通过 `session_refs_meta.last_scan_timestamp` 记录水位线，只处理新入库的消息。

##### 初次扫描结果（2026-04-25，25229 sessions）

| 指标 | 值 |
|------|---|
| 唯一引用边 | 3023 |
| 涉及的 session | 2189 (8.7%) |
| continuation 链 | 264 条（最长 14 sessions） |
| text 引用 | 2740 条 |
| 最被引用的 session | 13 次（keeta 项目） |
| 最高出度 session | 引用 39 个其他 session |

#### 输入与输出

**输入**：一个 session graph 中所有 session 的 L3 摘要 + L2 细节

**输出**：知识节点，每个节点包含：
- 结论/事实
- 置信度（从 graph 演化中计算，不需要人工标注）
- 来源追溯（哪些 session 的哪些 L2 贡献了这条知识）
- 演化历史（被确认/修正/推翻的记录）
- 状态（current / exploratory / reference / superseded / invalidated）

**触发时机**：新 session compact 到 L3 后，LLM 分析该 session 所属 graph 的历史，判断知识节点是新增/确认/修正/推翻。

**置信度涌现**：不需要人工打分。同一个 graph 内：
- 后续 session 确认 → 置信度上升
- 后续 session 推翻 → 旧结论置信度归零，新结论开始
- subagent 的发现可以交叉验证 → 多个 subagent 得出相同结论时置信度更高
- 长时间无后续 → 按项目活跃度衰减

**拆分粒度**：一个 session 可能涉及多个主题。LLM 的输入是 L2 序列，由 LLM 判断哪些 L2 属于同一个知识主题，分别归到对应的 L4 节点。orchestrate 的 subagent 通常聚焦单一子任务，天然适合作为 L4 节点的原子输入。

**候选匹配**：LLM 需要知道已有哪些 L4 节点。不能全量塞进 context，先用 embedding 召回候选 L4 节点，再让 LLM 判断是否匹配。

### L5：项目级技术图谱

L5 不是 pipeline 中的固定一层，而是**多源构建的视图层**。

#### 多源构建

L5 可以从多种来源生成，也可以混合使用：

| 来源 | 时机 | 特点 |
|------|------|------|
| 项目 README / 定义文档 | 项目初期 | 自顶向下，框架先行 |
| L4 nodes 自底向上聚合 | L4 积累到一定量后 | 数据驱动，反映实际探索 |
| 从特定 session 出发沿引用图展开 | 按需 | 聚焦某条探索路径 |
| 人工定义 | 任意时刻 | 领域专家判断 |
| 以上混合 | 迭代中 | 通常是最终形态 |

#### L4 ↔ L5 双向 bootstrap

L5 和 L4 不是单向的聚合关系，而是**互相指导**：

```
初期：L4 盲提（无框架，~6.8 nodes/session）
      → 聚合出 L5 草稿（自底向上）

中期：L5 草稿 → 指导 L4 重新提取（有框架，更精准，区分主次）
      → L4 反馈修正 L5（发现新域、合并冗余域）

稳态：L5 稳定 → L4 增量提取自动归位
      → L5 偶尔因新发现扩展
```

有 L5 框架时，L4 提取从"盲提一切"变成"有框架的提取"：
- 知道这个 session 的发现应该往哪个域靠
- 能区分核心知识和过程噪音
- 置信度给分更有依据

#### 输出

项目的技术全景 — 类似手写的 `architecture-baseline.md`，但它是**活的**，随新 session 自动演化。

包含：
- 项目的领域结构（顶层域 + 子主题）
- 各域下的核心经验和踩坑记录（来自 L4 nodes）
- 知识节点之间的关联关系
- 整体置信度分布
- 空白区域标记（L5 定义了域但 L4 还没有数据）

### 三层能力架构

基于 L0-L5 compact 体系，上层能力分三层，有依赖关系：

**能力层 1: Session 标注 + 搜索权重**

给 session 附加元数据（status + weight），结合项目活跃度衰减影响搜索排序。这是基础能力层。

**能力层 2: L4/L5 知识结晶**

沿 compact 管线延伸，从 L3 + chain 中结晶出主题知识（L4）和项目图谱（L5）。

**能力层 3: 跨人共享 + 推荐**

基于前两层，在中央 server 做跨人的 L4/L5 匹配和通知。这是最终目标但也是最难的部分。

---

## 三、搜索权重体系（首要）

### 核心原则：基于项目活跃节奏的时间衰减

**不是绝对时间衰减，而是相对于该项目的活跃度来衰减。**

同样是 30 天前的 session：
- 在日活跃项目（如 ETerm）里 → 已经很旧了，之后产生了大量新 session，权重该降
- 在低频项目（如 yin DSL）里 → 可能就是最近一次工作，权重应该高

#### 衰减基准

不用"距今多少天"，而是"这个项目在这之后又产生了多少活动"来归一化。

可用的信号（memex 已有数据，不需要额外标注）：
- 该项目在该 session 之后的 session 数量
- 该项目在该 session 之后的消息总量
- 该项目最近一次活跃时间
- 该项目的历史活跃频率分布

#### 衰减量化

使用相对位置（而非绝对时间）：

```
position = 该 session 之后该项目的 session 数 / 该项目总 session 数
```

position 0 = 最新，position 1 = 最老。相对数保证了不同活跃度的项目自然适配：
- 1000 session 的项目后面跟了 50 个不算什么
- 10 session 的项目后面跟了 5 个就已经很旧了

#### 最终搜索权重

```
score = relevance_score × decay(position, status_decay_rate) × weight
```

三因子相乘：
- `relevance_score` — FTS/向量搜索相关度
- `decay(position, status_decay_rate)` — 基于项目活跃度 + 状态衰减系数
- `weight` — 标注权重（0.0-1.0 连续值）

#### 项目级衰减配置（Profile）

不同项目性质不同，衰减策略应该可配。

**配置位置**：`{project_path}/.vimo/memex/profile.toml`

```toml
# keeta 逆向项目 — 探索性结论衰减快
[decay]
current = 0.5         # 默认衰减
exploratory = 0.9     # 探索性结论快速衰减
reference = 0.3       # 参考资料保留较久
superseded = 0.7
invalidated = 0.85
```

```toml
# yin DSL 设计项目 — 设计决策长期有效
[decay]
current = 0.1         # 几乎不衰减
exploratory = 0.4
reference = 0.2
superseded = 0.3
invalidated = 0.8
```

**读取策略**：

1. memex 采集 session 时，从 `~/.claude/projects/{encoded-path}/` 解码得到项目实际路径
2. 检查 `{project_path}/.vimo/memex/profile.toml` 是否存在
3. 存在 → 加载项目级配置
4. 不存在 / 读取失败 / 路径不可达 → 落回全局默认配置（`~/.vimo/memex/default-profile.toml`）
5. 全局默认也不存在 → 使用硬编码的内置默认值

**sync 到 server 时**：profile 随 session 数据一起推到中央，server 端不需要访问项目目录。

**配置可 git 跟踪**：团队成员 clone 项目后自动共享同一份衰减策略。

---

## 四、Session 标注

### 设计原则

1. **最小化** — 维度越多，被标注的概率越低
2. **不做分类体系** — 项目横跨 DSL 编译器、多端 App、网络引擎、设计系统、AI 工具链等完全不同的领域，任何预设分类都会过时
3. **时间衰减处理 80% 的问题** — 标注只解决剩下 20%

### 两个独立维度

标注拆分为两个正交维度，避免固定档位的粒度问题：

#### 1. 状态（定性）— 结论的当前效力

| 状态 | 含义 | 典型场景 |
|------|------|---------|
| `current` | 结论当前有效（默认） | 正常工作、功能开发、配置调整 |
| `exploratory` | 探索性结论，未定论 | 逆向分析进行中、技术调研、原型验证 |
| `reference` | 不是当前做法，但有参考价值 | 旧调研报告、被放弃但思路有用的方案 |
| `superseded` | 被新方案替代 | polling → 事件驱动、旧架构 → 新架构 |
| `invalidated` | 已证伪 / 不再适用 | 逆向时的错误判断、过时的环境假设 |

状态影响衰减速率（通过项目级 profile 配置），但**不会从搜索池中移除** — 所有状态都可被搜到，支持 filter 精确查询。

#### 2. 权重（定量）— 连续值 0.0-1.0

独立于状态的重要性调节。默认 0.5。

同一状态的 session 重要性可以不同：
- `current` + weight 0.9 → yin DSL 语法设计（重要决策）
- `current` + weight 0.3 → 一次日常配置调整
- `superseded` + weight 0.7 → 旧方案本身有参考价值
- `superseded` + weight 0.2 → 完全没用的弯路

### 数据模型

```sql
CREATE TABLE session_annotations (
    session_id    TEXT PRIMARY KEY,
    status        TEXT NOT NULL DEFAULT 'current',
    weight        REAL NOT NULL DEFAULT 0.5,    -- 0.0-1.0
    reason        TEXT,                          -- 标注原因
    superseded_by TEXT,                          -- 被哪个 session 替代（status=superseded 时可选填）
    tags          TEXT,                          -- JSON 数组，自由标签
    annotated_by  TEXT NOT NULL DEFAULT 'user',  -- user / llm
    annotated_at  INTEGER NOT NULL,              -- 标注时间戳（ms）
    FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);
```

---

## 五、三层接口

标注能力通过三层接口暴露，每层独立，逐层包装：

### 1. API（底层）

REST 接口，memex server 本身的能力。

```
PUT    /api/sessions/:id/annotation   — 写入/更新标注（status, weight, reason, tags）
GET    /api/sessions/:id/annotation   — 读取标注
DELETE /api/sessions/:id/annotation   — 清除标注（回到默认 current/0.5）
GET    /api/sessions?status=superseded          — 按状态过滤
GET    /api/sessions?status!=invalidated        — 排除某状态
GET    /api/sessions?weight_min=0.7             — 按权重过滤
```

### 2. MCP Tool（中层）

包装 API，暴露给 AI 对话使用。

```
annotate_session(session_id, status?, weight?, reason?, tags?)
  — 标注 session，所有字段可选更新

get_annotation(session_id)
  — 查询标注状态
```

使用场景：
- 用户："这个 session 的结论是错的" → AI 调 `annotate_session(id, status="invalidated", reason="...")`
- 用户："keeta 那个逆向结论不太确定" → AI 调 `annotate_session(id, status="exploratory", weight=0.3)`
- 用户："yin 的语法设计很重要，钉住" → AI 调 `annotate_session(id, weight=0.95)`

### 3. LLM 自动标注（上层，可选）

挂接 LLM（通过 Athanor 统一适配），对 session 做自动分析和标注建议。

- **默认关闭** — 功能必须存在，但用户自行决定是否启用
- **开关**：项目级 profile 或全局配置中 `[auto_annotate] enabled = true`
- **触发时机**：compact L3 完成后自动触发
- **输入**：L3 摘要
- **输出**：建议的 status + weight + 理由
- **LLM 选型**：走 Athanor 统一适配，不绑定特定模型
- **来源标记**：`annotated_by = "llm"`，与用户标注区分，用户可随时覆盖

可以：
- 批量跑历史 session
- 在新 session compact 完成后自动触发
- 人工确认后写入

### 标注触发时机

| 方式 | 覆盖率 | 精度 | 场景 |
|------|--------|------|------|
| 对话中主动标注 | 低 | 最高 | 用户说"标记一下"，AI 自动填 session_id 调 MCP |
| 回溯修正 | 中 | 高 | 搜到旧 session 发现结论过时，当场改标注 |
| LLM 自动标注 | 高 | 待验证 | compact L3 后自动分析，默认关闭 |

大部分 session 不会被标注 → 全靠默认值（`current` + `0.5`）+ 项目活跃度衰减兜底。标注体验要足够轻 — 用户只需说一句话，AI 自动处理 session_id 和参数填充。

---

## 六、跨人推荐（远期）

### 已确认的难点

1. **语义相似 ≠ 该推荐** — ETerm 做字体渲染和 shuimo-core 做 fontkit 替换，都有"font"关键词但完全不相关，推荐是噪音
2. **LLM 判断成本高** — 要给 LLM 足够上下文才能准确判断，但每次 push 都跑 LLM 不现实
3. **去噪难** — 同一个人在同一个项目的连续推进不应触发推荐

### 前置依赖

- Session 标注（Layer 1）— 知道哪些 session 值得参与匹配
- 项目活跃度权重 — 区分"新工作"和"日常推进"
- 知识沉淀（Layer 2）— 匹配的是提炼后的知识，不是原始 session

### 可能的渐进路线

**Phase 0（现在就能做）**：`source: "remote"` 编译上线，手动查询

**Phase 1**：搜索结果融合项目活跃度权重 + pin/dismiss 标注

**Phase 2**：中央 server 被动匹配 — push 进来时异步入队，做 session 级别相似度匹配，但不自动推送，只在用户主动查询时附带提示

**Phase 3**：主动推荐 — 高匹配度时推送通知

---

## 七、竞品参考：MemPalace

### 架构对比

| | memex | MemPalace |
|---|---|---|
| 语言 | Rust | Python |
| 向量引擎 | LanceDB（嵌入式） | ChromaDB（HNSW bloat 问题） |
| Embedding | bge-m3 1024d | MiniLM 384d |
| 搜索 | FTS5 + 向量混合 | BM25 + 向量混合 |
| 分片 | 2000 char，代码/文本分离 | 800 char，不区分 |
| 压缩 | L0-L3 四级 compact | 无（声称 verbatim，实际 closet 层有压缩索引） |
| RAG | 有，带上下文窗口 | 无内置 |
| 多人同步 | 有（已跑通） | 无（规划中） |
| 知识组织 | 无（本文档要解决的） | wings/rooms/closets/drawers 空间隐喻 |

### MemPalace 所谓"无压缩"的实际情况

- **Drawer**：原文按 800 char 切块存 ChromaDB → 确实是原文
- **Closet**：用 LLM（可选）或正则提取 topics/quotes/summary 作为索引 → 压缩
- **Layer 0-1**：wake-up 时只加载 ~600-900 tokens → 压缩

本质是**存储层保留原文，索引层做压缩**。与 memex 的 L0 原文 + L1-L3 逐级压缩思路类似。

### 可借鉴的点

- Closet 的 AAAK 压缩格式（AI 可读的索引卡片）作为知识沉淀层的参考
- 分层加载策略（Layer 0-3 按需加载，不一次全塞进去）

### 不采用的点

- ChromaDB — LanceDB 更适合嵌入式场景
- 空间隐喻（wings/rooms）— 命名友好但增加认知负担，memex 用 project/session 已经够用
- 800 char 小分片 — 丢失上下文，memex 的 2000 char + 代码分离更合理

---

## 八、现有能力盘点

memex 已有但未充分利用的能力：

| 能力 | 状态 | 备注 |
|------|------|------|
| FTS5 全文搜索 | ✅ 在用 | MCP search_history 默认路径 |
| LanceDB 向量搜索 | ✅ 已实现 | bge-m3 1024d |
| 混合搜索（FTS + 向量） | ✅ 已实现 | HybridSearchService |
| RAG 问答 | ✅ 已实现 | RagService，带上下文窗口 |
| L0-L3 compact | ✅ 在用 | Qwen3 0.6B 本地 |
| 多人 sync | ✅ 在用 | 事件驱动，NAS server |
| `source: "remote"` MCP 参数 | ⚠️ 代码已有，未编译上线 | search_history 可查远端 |
| 项目活跃度权重 | ❌ 未实现 | 本文档首要项 |
| Session 标注 | ❌ 未实现 | 本文档核心项 |
| L4 主题级知识 | ❌ 未实现 | compact 体系延伸 |
| L5 项目技术图谱 | ❌ 未实现 | L4 之上的再压缩 |
| 会话链（chain） | ✅ 已有（已修复） | continuation_chain，覆盖率从 5% 提升到 95% |
| 子会话关系（orchestrate） | ✅ 已有 | session_relations，4390 条 subagent 关系 |

---

## 九、实验发现（2026-04-25）

### Chain 修复

`read_continuation_from_jsonl` 只检查第一条 user 消息，但 96% 的 `/continue` marker 在第 3 条。已修复为前 5 条，DB 回填后覆盖率从 5% → 95%（24 chains → 119 chains）。

### Session 关系三种类型

| 关系 | 触发 | 存储 | 状态 |
|---|---|---|---|
| continuation | `/continue` | `continuation_chain_nodes` | ✅ 已修复 |
| orchestrate | `/orchestrate` | `session_relations` | ✅ 已有 |
| reference | 用户贴 session ID（如 `6668d883`） | ❌ 未捕获 | 待实现 |

reference 关系检测：扫 user 消息正则匹配短 UUID 前缀，确认 DB 有对应 session 即建立引用。

### L4 输入与模型选型

**问题**：L1-L3 全由 Qwen3 0.6B 生成，信息保真度不够支撑 L4（"PuLID 和 Kontext 不兼容"被压成"integrated Kontext and PuLID"）。

**实验过程**：

1. **0.6B 做 turn 级分类** — 失败。instruction following 能力不够，输出空白或乱码
2. **0.6B 做 session 级提取** — 60-70% 可用，但偏"做了什么"而非"发现了什么"
3. **8B（本地 Mac）做定性** — 质量明显好于 0.6B，但 M3 Max 上转风扇
4. **14B（5090 远端）做定性** — 质量最好，TAGS 精准、KEY_TURNS 有选择性
5. **14B 两步 pipeline** — Pass 1 自由格式拿质量，Pass 2 收敛到 JSON schema，效果最好

**结论**：L4 从 L0 直接提取，不依赖 L1-L3 质量。使用 5090 上的 Qwen3 14B 跑两步 pipeline。

```
L0 原始消息 (from DB, human_input + assistant only)
        ↓
  5090 Qwen3 14B — Pass 1: 自由格式分析
        ↓
  5090 Qwen3 14B — Pass 2: 收敛到 JSON schema
        ↓
  L4 知识节点
```

**性能指标**（5090D，Qwen3 14B，keeta 7K session）：
- 单 session 两步合计：~15 秒
- 推理速度：~104 tokens/s
- GPU 显存占用：15.7GB / 24GB（常驻）
- 全量 25K sessions 估算：~104 小时（可后台批量跑）

**两步 Prompt 设计**：

Pass 1（自由格式，拿质量）：
- WHAT: 一句话概括 session
- KEYWORDS: 3-5 个领域术语
- VALUABLE: 最重要的结论/决策（max 5）
- WORTH_EXTRACTING: yes/maybe/no

Pass 2（收敛，拿结构）：
- 基于 Pass 1 的分析，填入 JSON schema
- knowledge_nodes 数组：topic, conclusion, type(decision/discovery/pitfall/reference), confidence

**为什么不用 0.6B 预筛**：session 中位数才 ~2.3K tokens，直接喂 14B 完全可控，加预筛反而增加复杂度且 0.6B 分类质量不够。

### L4 Pilot 实验（keeta 项目，12 sessions）

在 keeta 项目上选 12 个 session（含 3 条 chain + 4 个 standalone），跑两步 pipeline，产出 60 个知识节点。

**产出统计**：
- 60 nodes: 34 discovery, 18 decision, 6 pitfall, 2 reference
- Confidence 分布: min=0.7, avg=0.89, max=1.0
- 每 session 平均 5 个节点

**跨 session 聚类**（手动关键词匹配）：

| 主题 | 节点数 | 跨 session 数 | 特征 |
|---|---|---|---|
| a7 生成/序列化 | 14 | 4 | 结论演化：从"不可逆非线性变换"到"无密码学变换" |
| DFP 加密/key_C | 9 | 5 | 反复确认：per-session random key_C |
| a2/GF(2)/HMAC 签名 | 3 | 2 | **被推翻**：chain-B 说 GF(2)/WB-AES 可行，chain-C 证伪 |
| a9 结构/密码 | 4 | 1 | 高置信度完整验证（13334/13334 samples） |
| a5 算法 | 2 | 1 | 完全验证通过 |
| WireGuard/网络 | 4 | 1 | 架构决策集中 |
| VM/Ghidra 分析 | 4 | 1 | 工具踩坑为主 |

**关键发现**：
1. **知识演化可追踪** — 同一主题跨 session 出现确认/修正/推翻关系
2. **Chain 内演化最明显** — chain-A 的 a7 分析从浅到深，chain-B 的 DFP 从可行到卡 RSA
3. **矛盾可检测** — a2 签名在 chain-B 和 chain-C 之间被推翻，14B 都标出了具体结论
4. **Standalone 补充 chain 盲区** — WireGuard、Ghidra 等独立 session 的知识不在任何 chain 上

### messages 表 type 分类问题

`type='user'` 混了多种完全不同的内容。全局 518K 条 user 消息中 79%（410K）是 tool result。

需要加 `subtype` 字段区分：

| content 特征 | subtype |
|---|---|
| `content` 为 string | `human_input` |
| `content` array 含 `tool_result` | `tool_result` |
| 含 `<command-name>` 标签 | `command` |
| 含 `<system-reminder>` | `system_reminder` |
| 含 `<task-notification>` | `task_notification` |

raw 字段有足够信息做判断，入库时解析即可。

---

## 十、TODO

### 近期（能力层 1）
- [ ] 定义项目活跃度衰减函数（相对位置 + 项目 profile）
- [ ] 设计标注数据模型（status 五态 + weight 连续值）
- [ ] 标注 API + MCP tool
- [ ] `source: "remote"` 编译上线

### 近期（数据质量）
- [x] continuation chain 修复（`read_continuation_from_jsonl` 前 5 条 user 消息）
- [ ] messages 表增加 `subtype` 字段 + 入库时分类
- [x] session reference 关系检测 + 存储（`session_refs` 统一表，3023 edges）

### 中期（能力层 2）
- [ ] L4 pipeline 工程化：L0 → 5090 Qwen3 14B 两步提取 → knowledge_nodes 表
- [ ] L4 知识节点数据模型设计（已有 pilot JSON schema）
- [ ] L4 跨 session matching：embedding 召回 → 确认/修正/推翻判断
- [ ] L4 置信度演化：跨 session 确认次数 + 被推翻检测
- [ ] L5 项目图谱生成：聚合 L4 节点 → 项目全景

### 远期（能力层 3）
- [ ] 跨人 L4/L5 匹配
- [ ] 推荐去噪策略
- [ ] 通知通道设计
