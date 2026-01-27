# Project Alias 设计文档

> 状态：草案
> 创建：2025-01-27
> 更新：2025-01-27
> 背景：解决项目路径变更导致会话历史割裂 + Gemini hash 无法映射的问题

## 问题背景

### 当前问题

1. **项目路径变更导致历史割裂**
   - 用户把项目从 `/old/path` 移动到 `/new/path`
   - 旧会话在 "项目A"，新会话在 "项目B"
   - 本质是同一个项目，但数据库中分裂了

2. **Gemini CLI 的 hash 无法映射**
   - Gemini 用 `SHA256(绝对路径)` 作为 projectHash，不存储原始路径
   - 数据库中显示为 `Project-00bec3fa` 这种无意义名称
   - 无法从 hash 反推路径（SHA256 单向）

### 根本原因

当前 `projects` 表以路径为主键，**路径 = 项目身份**。但实际上：
- 路径会变（移动、重命名）
- 不同工具用不同标识符（Claude 用路径，Gemini 用 hash）

## 设计方案：Alias 机制

### 核心思想

**项目身份 ≠ 路径**

一个逻辑项目可以有多个标识符（alias）：
- 旧路径、新路径
- Gemini hash（移动前后会产生不同 hash）

### 数据模型

```sql
-- 逻辑项目表
CREATE TABLE projects (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,              -- 显示名称（用户可编辑）
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- 项目别名表
CREATE TABLE project_aliases (
    id INTEGER PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    alias TEXT NOT NULL,             -- 路径 或 hash
    alias_type TEXT NOT NULL,        -- 'path' | 'hash'
    source TEXT NOT NULL,            -- 'claude' | 'gemini' | 'codex' | 'user'
    is_canonical BOOLEAN DEFAULT 0,  -- 是否是主标识（用于显示）
    created_at INTEGER NOT NULL,

    -- 同类型的 alias 全局唯一（一个路径/hash 只能属于一个项目）
    UNIQUE(alias, alias_type)
);

-- 索引
CREATE INDEX idx_aliases_alias ON project_aliases(alias);
CREATE INDEX idx_aliases_project ON project_aliases(project_id);

-- 每个项目只能有一个 canonical alias
CREATE UNIQUE INDEX idx_aliases_canonical
ON project_aliases(project_id) WHERE is_canonical = 1;
```

### 约束说明

| 约束 | 目的 |
|------|------|
| `UNIQUE(alias, alias_type)` | 同一路径/hash 只能属于一个项目，避免歧义 |
| `UNIQUE(project_id) WHERE is_canonical=1` | 每个项目只有一个主标识用于显示 |
| `ON DELETE CASCADE` | 删除项目时自动清理 alias |

### 路径规范化

CLI 工具给的已经是绝对路径，只需处理跨平台差异：

```rust
fn normalize_path(path: &str) -> String {
    path
        .replace('\\', "/")           // Windows 斜杠转 Unix
        .trim_end_matches('/')        // 去尾部斜杠
        .to_string()
}
```

| 问题 | 处理方式 |
|------|----------|
| 斜杠方向 | 统一转为 `/` |
| 尾部斜杠 | 统一去除 |
| 大小写 | 存原样，macOS/Windows 查询时用 `COLLATE NOCASE` |
| 符号链接 | 不展开（保持用户视角） |

### Gemini Hash 说明

```
projectHash = SHA256(绝对路径)  // 不带尾部斜杠

示例：
/Users/test/calendar → 00bec3fa1d026cc3728eb8d722c54ead3dbfea2bbb817fab5ece4d1cea5462ad
```

**无法反推**，映射方式：
1. 用户手动指定路径（会校验 SHA256 匹配）
2. 用户手动关联到已有项目
3. 从会话内容推断提示（辅助）

## 场景处理

### 场景 1: Claude 新会话（新项目）

```
输入: cwd = '/Users/test/new-project'

1. normalize(cwd)
2. 查 project_aliases WHERE alias = ? AND alias_type = 'path'
3. 没找到 → 创建新项目 + alias (is_canonical=1)
4. 会话归属到新项目
```

### 场景 2: Claude 新会话（已有项目）

```
输入: cwd = '/Users/test/calendar'

1. normalize(cwd)
2. 查 project_aliases → 找到 project_id
3. 会话归属到该项目
```

### 场景 3: Gemini 新会话（未映射 hash）

```
输入: projectHash = 'abc123...'

1. 查 project_aliases WHERE alias = ? AND alias_type = 'hash'
2. 没找到 → 创建新项目 (name='Project-abc123') + alias
3. UI 标记为"待映射"
```

### 场景 4: 用户映射 Gemini hash

**方式 A: 关联到已有项目**

```sql
BEGIN TRANSACTION;

-- 1. 转移 alias 到目标项目
UPDATE project_aliases
SET project_id = :target_id, is_canonical = 0
WHERE alias = :hash AND alias_type = 'hash';

-- 2. 转移 sessions
UPDATE sessions SET project_id = :target_id WHERE project_id = :old_id;

-- 3. 删除空项目（如果没有其他 alias）
DELETE FROM projects WHERE id = :old_id
AND NOT EXISTS (SELECT 1 FROM project_aliases WHERE project_id = :old_id);

COMMIT;
```

**方式 B: 指定路径映射**

```
用户输入: /Users/test/calendar

1. 校验: SHA256(normalize(输入)) == hash
2. 校验通过 →
   - UPDATE projects SET name = 'calendar'
   - INSERT project_aliases (alias=路径, alias_type='path', is_canonical=1)
   - UPDATE project_aliases SET is_canonical=0 WHERE alias=hash
```

### 场景 5: 合并项目（目录移动后）

```
情况:
- 项目 A: /old/calendar (100 sessions)
- 项目 B: /new/calendar (50 sessions)
用户确认是同一个项目，合并 A → B
```

```sql
BEGIN TRANSACTION;

-- 1. 转移 alias（取消 A 的 canonical，保留 B 的）
UPDATE project_aliases
SET project_id = :B_id, is_canonical = 0
WHERE project_id = :A_id;

-- 2. 转移 sessions
UPDATE sessions SET project_id = :B_id WHERE project_id = :A_id;

-- 3. 删除项目 A
DELETE FROM projects WHERE id = :A_id;

COMMIT;
```

### 场景 6: 查询

```sql
-- 通过 project_id 直接查（推荐）
SELECT * FROM sessions WHERE project_id = ?

-- 通过 alias 查
SELECT s.* FROM sessions s
JOIN project_aliases pa ON s.project_id = pa.project_id
WHERE pa.alias = ? AND pa.alias_type = 'path'
```

## API 设计

```rust
trait ProjectAliasService {
    /// 通过 alias 查找项目（自动 normalize）
    fn find_by_alias(&self, alias: &str, alias_type: AliasType) -> Option<Project>;

    /// 获取或创建项目（collector 用）
    fn get_or_create(&self, alias: &str, alias_type: AliasType, source: &str) -> Result<i64>;

    /// 给项目添加 alias
    fn add_alias(&self, project_id: i64, alias: &str, alias_type: AliasType, source: &str) -> Result<()>;

    /// 合并项目（事务）
    fn merge(&self, from_id: i64, to_id: i64) -> Result<()>;

    /// 列出未映射的 Gemini 项目
    fn list_unmapped(&self) -> Vec<UnmappedProject>;

    /// 设置 canonical alias
    fn set_canonical(&self, project_id: i64, alias_id: i64) -> Result<()>;
}

enum AliasType { Path, Hash }

struct UnmappedProject {
    id: i64,
    hash: String,
    display_name: String,      // "Project-abc123"
    session_count: i64,
    last_active: i64,
    hint: Option<String>,      // 从会话内容推断的可能路径
}
```

## 边界情况

| 情况 | 处理 |
|------|------|
| alias 冲突（已属于其他项目） | 拒绝，提示用户先合并 |
| 项目没有 alias | 不允许删除最后一个 alias |
| 合并后 canonical 选哪个 | 保留目标项目的 canonical |
| hash 映射后 canonical 切换 | 自动把 path 设为 canonical，hash 降为普通 alias |

## 迁移计划

### 现有 Schema

```sql
-- 当前 projects 表
projects (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,  -- 路径 或 'gemini:hash' 格式
    name TEXT NOT NULL,
    source TEXT NOT NULL,       -- 'claude' | 'gemini' | ...
    encoded_dir_name TEXT,
    created_at INTEGER,
    updated_at INTEGER
)

-- sessions 通过 project_id (INTEGER) 关联，不需要改
```

### 渐进迁移策略

| 阶段 | 操作 | `projects.path` | 查询逻辑 |
|------|------|-----------------|----------|
| Phase 1 | 创建 `project_aliases` + 迁移数据 | 保留 | 优先 aliases，fallback path |
| Phase 2 | 代码全部改用 aliases | 保留（冗余） | 只用 aliases |
| Phase 3 | 移除 `projects.path`（可选） | 删除 | 只用 aliases |

**建议**：Phase 1 + 2 即可，Phase 3 可以不做。

### Migration 003: Project Aliases

```rust
const MIGRATION_VERSION: i64 = 3;

fn migration_003_project_aliases(conn: &Connection) -> SqliteResult<()> {
    info!("Running migration 003: Project aliases");

    // 1. 创建 project_aliases 表
    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS project_aliases (
            id INTEGER PRIMARY KEY,
            project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
            alias TEXT NOT NULL,
            alias_type TEXT NOT NULL,
            source TEXT NOT NULL,
            is_canonical BOOLEAN DEFAULT 0,
            created_at INTEGER NOT NULL,
            UNIQUE(alias, alias_type)
        );

        CREATE INDEX IF NOT EXISTS idx_aliases_alias ON project_aliases(alias);
        CREATE INDEX IF NOT EXISTS idx_aliases_project ON project_aliases(project_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_aliases_canonical
        ON project_aliases(project_id) WHERE is_canonical = 1;
    "#)?;

    // 2. 迁移普通路径
    conn.execute(r#"
        INSERT OR IGNORE INTO project_aliases
            (project_id, alias, alias_type, source, is_canonical, created_at)
        SELECT id, path, 'path', COALESCE(source, 'claude'), 1, created_at
        FROM projects
        WHERE path IS NOT NULL AND path NOT LIKE 'gemini:%'
    "#, [])?;

    // 3. 迁移 Gemini hash（去掉 gemini: 前缀）
    conn.execute(r#"
        INSERT OR IGNORE INTO project_aliases
            (project_id, alias, alias_type, source, is_canonical, created_at)
        SELECT id, REPLACE(path, 'gemini:', ''), 'hash', 'gemini', 1, created_at
        FROM projects
        WHERE path LIKE 'gemini:%'
    "#, [])?;

    info!("Migration 003 complete");
    Ok(())
}
```

### 回滚方案

```sql
-- 出问题时直接删除新表，现有数据不受影响
DROP TABLE IF EXISTS project_aliases;
DELETE FROM schema_migrations WHERE version = 3;
```

### 代码适配

1. **Collector**: 写入时用 `get_or_create(alias, type, source)`
2. **查询**: 通过 `project_id` 关联，无需改动
3. **UI**: 展示 canonical alias；未映射项目标记提示

## 未来扩展

- **Git remote URL**: 跨设备识别同一仓库
- **自动合并建议**: 检测相似项目名，提示用户
- **alias_type 扩展**: 支持 `label`（用户自定义标签）

## 参考

- Gemini CLI: `~/.gemini/tmp/{hash}/chats/session-*.json`
- Claude CLI: `~/.claude/projects/{encoded-path}/{session-id}.jsonl`
