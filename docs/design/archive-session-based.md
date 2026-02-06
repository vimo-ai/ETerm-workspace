# Archive V2: Session-Based 归档

> 创建时间: 2026-02-06
> 状态: 待实施
> 代码位置: `memex/memex-rs/src/archive/`

## 1. 问题

### 1.1 Subagent 数据丢失

Claude Code 的会话结构：

```
~/.claude/projects/{encoded-project}/
├── {uuid}.jsonl              ← 主会话
├── {uuid}/
│   └── subagents/
│       ├── agent-001.jsonl   ← subagent 会话
│       ├── agent-002.jsonl
│       └── ...
└── ...
```

当前 `find_archivable_files`（mod.rs:460-466）只做单层 `read_dir`：

```rust
for file_entry in std::fs::read_dir(&project_dir)? {
    let file_entry = file_entry?;
    let file_path = file_entry.path();
    if !file_path.is_file() {   // ← 跳过所有目录，包括 {uuid}/subagents/
        continue;
    }
}
```

**后果**：subagent 的 `.jsonl` 文件永远不会被归档。Claude 30 天后自动清理 JSONL，未归档数据永久丢失。

### 1.2 数据规模

全量扫描 `~/.claude/projects/` 发现：

- 474 个会话拥有 subagent 目录
- agent session 占总会话的 **72%**（2,655 / 3,661）
- 意味着当前归档体系遗漏了主要数据

### 1.3 Tar 结构扁平化

`compress_files`（compressor.rs:88-93）使用 `-C dir file_name` 打包：

```rust
for file in files {
    let dir = file_path.parent().unwrap();
    let name = file_path.file_name().unwrap();
    tar_cmd.arg("-C").arg(dir).arg(name);  // 只取 file_name，丢失目录结构
}
```

**后果**：

1. 归档中所有文件被打平到 tar 根目录
2. 无法直接还原到原始目录结构
3. 不同项目/会话的同名文件可能冲突

### 1.4 验证逻辑缺陷

`do_archive`（mod.rs:517-519）校验时只用 `file_name()`：

```rust
let file_name = original_path.file_name().unwrap();
let extracted_path = verify_dir.join(file_name);
```

如果保留目录结构，此处会找不到解压后的文件。

## 2. 设计原则

- **Session 是归档原子单位**：主 `.jsonl` + 所有 subagent 一起归档或不归档
- **保留原始目录结构**：tar 内保持 `{project}/{uuid}.jsonl` + `{project}/{uuid}/subagents/*.jsonl` 结构
- **可直接还原**：`tar -xf archive.tar.xz -C ~/.claude/projects/` 即可恢复
- **向后兼容**：合并（merge）阶段能处理旧的扁平归档

## 3. 核心概念：Effective Mtime

### 3.1 为什么不能只看主 session 的 mtime

假设：主 session mtime >= 所有 subagent mtime（主会话最后关闭）。

**数据验证推翻了这个假设**：

| 指标 | 数据 |
|------|------|
| 总会话数（有 subagent） | 474 |
| 违反假设的会话数 | 178 (**37.5%**) |
| 典型偏差 | 1~18 秒 |
| 极端偏差 | 最大 20 小时 |

原因：Claude Code 的 subagent 可能在主会话写入完毕后才完成最后一次写入（后台任务、异步 tool call 等）。

### 3.2 定义

```
effective_mtime(session) = max(
    mtime(main.jsonl),
    max(mtime(subagent_i.jsonl) for all subagents)
)
```

所有需要 mtime 的地方——静默期检查、日期分组——统一使用 effective mtime。

## 4. 新数据结构

### 4.1 ArchivableSession

```rust
/// 可归档的会话单元
struct ArchivableSession {
    /// 会话 UUID
    /// - 有主文件时：从 {uuid}.jsonl 文件名取
    /// - 孤立 subagent 时：从目录名 {uuid}/subagents/ 推导
    uuid: String,
    /// 项目目录（encoded project path）
    project_dir: String,
    /// 主 session 文件的相对路径（从 source_dir 起算）
    /// None 表示孤立 subagent（主文件已被 Claude 清理）
    /// e.g. Some("-Users-foo-project/{uuid}.jsonl")
    main_file: Option<PathBuf>,
    /// subagent 文件的相对路径列表（仅 .jsonl 文件）
    /// e.g. ["-Users-foo-project/{uuid}/subagents/agent-001.jsonl", ...]
    subagent_files: Vec<PathBuf>,
    /// effective mtime = max(main, all subagents)
    effective_mtime: SystemTime,
}

impl ArchivableSession {
    /// 所有文件的相对路径（main + subagents）
    fn all_relative_paths(&self) -> Vec<&Path> { ... }

    /// effective mtime 对应的日期
    fn effective_date(&self) -> NaiveDate { ... }

    /// 文件总数
    fn file_count(&self) -> usize { ... }
}
```

### 4.2 Tar 内部结构

```
archive.tar.xz
├── -Users-foo-project/
│   ├── abc123.jsonl                    ← 主会话
│   ├── abc123/
│   │   └── subagents/
│   │       ├── agent-001.jsonl
│   │       └── agent-002.jsonl
│   ├── def456.jsonl                    ← 另一个会话（无 subagent）
│   └── ...
├── -Users-bar-project/
│   └── ...
└── ...
```

还原命令：`tar -xJf archive.tar.xz -C ~/.claude/projects/`

## 5. 实现变更

### 5.1 `find_archivable_files` → `find_archivable_sessions`

**当前**：返回 `Vec<PathBuf>`（扁平文件列表）
**改为**：返回 `Vec<ArchivableSession>`

```
遍历 source_dir/
  遍历 {project_dir}/
    对每个 {uuid}.jsonl:
      收集 main_file = {project_dir}/{uuid}.jsonl
      如果存在 {project_dir}/{uuid}/subagents/:
        收集所有 subagent .jsonl 文件
      计算 effective_mtime = max(all files)
      检查 quiet_period（用 effective_mtime）
      检查 effective_date == target_date
      生成 ArchivableSession（路径均为相对于 source_dir 的相对路径）
```

**扫描规则**：
- 仅收集 `.jsonl` 文件，跳过 subagent 目录中的非 JSONL 文件
- UUID 目录只读取 `subagents/` 子目录，忽略其他内容（兼容未来扩展）
- 空 subagent 目录（存在 `{uuid}/subagents/` 但无 `.jsonl`）：如果有主文件，照常归档主文件；如果主文件也没有，跳过
- 所有 relative path 必须来自 `strip_prefix(source_dir)`，拒绝包含 `..` 的路径（防止路径逃逸）

**孤立 subagent 目录处理**：如果存在 `{uuid}/subagents/` 但没有 `{uuid}.jsonl`（主文件已被 Claude 清理），仍然作为一个 session 归档。此时 `main_file` 为 `None`，`uuid` 从目录名推导，effective_mtime 取 subagent 的最大值。

### 5.2 `compress_files` → `compress_with_structure`

**当前**：接受 `&[impl AsRef<Path>]`（绝对路径列表），`-C dir file_name` 打平
**改为**：接受 `(base_dir: &Path, relative_paths: &[PathBuf])`

```rust
// 新签名
pub fn compress_with_structure(
    &self,
    base_dir: &Path,           // ~/.claude/projects/
    relative_paths: &[PathBuf], // ["-Users-foo/abc.jsonl", "-Users-foo/abc/subagents/agent-001.jsonl", ...]
    output: &Path,
) -> Result<()>
```

Tar 命令：
```bash
tar -cf output.tar -C {base_dir} -- {rel_path_1} {rel_path_2} ...
```

**关键：必须加 `--`**。Claude 的项目路径编码以 `-` 开头（如 `-Users-foo-project/`），不加 `--` 会被 tar 解析为选项，导致打包失败。

一次 `-C` 切换到 base_dir，后续所有路径都是相对路径，目录结构自然保留。

**路径数量过多时**：如果相对路径列表超过 ARG_MAX 限制，改用 `tar -T filelist.txt`（文件列表模式）。初期可不实现，作为后续优化。

### 5.3 `do_archive` 验证逻辑

**当前**：`verify_dir.join(file_name)` 查找解压文件
**改为**：`verify_dir.join(relative_path)` 查找

```rust
for (relative_path, original_hash, _) in &original_hashes {
    let extracted_path = verify_dir.join(relative_path);
    // ... SHA-256 校验
}
```

### 5.4 `do_merge` 递归收集

**当前**：`read_dir(&merge_dir)` 只收集顶层文件
**改为**：递归 `walkdir` 收集所有文件，保留相对路径

```rust
// 解压所有归档到隔离子目录（防止不同归档间文件覆盖）
for (i, archive) in archives.iter().enumerate() {
    let sub_dir = merge_dir.join(format!("{}", i));
    self.compressor.decompress(archive, &sub_dir)?;
}

// 递归收集，去重（相同相对路径只保留一份）
let mut seen = HashSet::new();
let mut all_relative = Vec::new();

for sub_dir in sub_dirs {
    for entry in walkdir::WalkDir::new(&sub_dir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(&sub_dir)?;
            if seen.insert(rel.to_path_buf()) {
                all_relative.push(rel.to_path_buf());
            }
        }
    }
}

// 复制去重后的文件到统一目录，保持目录结构
// 然后用 compress_with_structure 打包
```

**为什么需要隔离子目录**：不同归档包可能包含相同会话的文件（跨日期边界的 session），直接解压到同一目录会相互覆盖。

**去重与冲突策略**：
- 归档列表在 merge 前按路径名排序（保证确定性），即按时间升序
- 相同 relative path 只保留最先遇到的（更早日期的文件优先）
- JSONL 是 append-only，同一文件不应出现在多个日包中（仅在补偿归档边界场景可能重复）
- 如果同路径不同 hash，记录 warn 日志但不中断（保留先遇到的版本，更早的包通常更完整）

### 5.5 旧归档兼容

旧的扁平归档在 merge 时会被解压到隔离子目录。由于它们没有目录结构（所有 `.jsonl` 都在根目录），合并后会呈现为：

```
merged.tar.xz
├── -Users-foo-project/
│   ├── new-session.jsonl           ← 新格式：有目录前缀
│   └── new-session/subagents/...
├── old-session.jsonl               ← 旧格式：扁平在根目录
└── ...
```

这是可接受的。旧数据本身就没有 subagent，结构兼容不会丢数据。

**注意**：旧扁平归档还原时（`tar -xJf -C ~/.claude/projects/`）会将 `.jsonl` 解压到 `projects/` 根目录而非 project 子目录。这是历史行为，不影响功能。未来如需整理，可以写一次性迁移脚本根据文件名匹配 DB 中的 project 路径，移到正确位置。

## 6. 边缘场景

### 6.1 跨日期边界的 Session

场景：主文件 mtime = 1月15日 23:58，subagent mtime = 1月16日 00:03。

处理：effective_mtime = 1月16日 → 归入 1月16日的日包。整个 session（包括主文件）打入同一个包。

**不存在拆分**：一个 session 的所有文件永远在同一个归档包中。

### 6.2 孤立 Subagent 目录

场景：`{uuid}/subagents/agent-001.jsonl` 存在，但 `{uuid}.jsonl` 不存在（Claude 先清理了主文件）。

处理：照常归档。`main_file` 为 `None`，effective_mtime 取 subagent 最大值。还原后不影响数据完整性——主文件数据已在 DB 中。

### 6.3 活跃 Session 部分文件已静默

场景：subagent-001 已静默 48h，但主 session 仍在写入。

处理：effective_mtime 取最大值（主 session 的 mtime），因此整个 session 不满足 quiet_period，不会被归档。**只有所有文件都静默超过 24h，session 才会被归档**。

### 6.4 归档期间并发写入

场景：quiet_period 检查通过后、tar 打包过程中，Claude 恰好重新写入该 session。

处理：这是已有逻辑的固有 race，session-based 方案没有加剧。24h quiet_period 使此概率极低。如果 SHA-256 验证阶段发现 hash 不匹配（文件被写入），归档失败并回滚。下次重试时该 session 的 effective_mtime 已更新，quiet_period 重新计算。

### 6.5 超大 Session

场景：一个 session 有 100+ subagent，文件总计数 GB。

处理：正常归档。xz -9e -T0 对 JSONL（高度结构化文本）压缩率极高（通常 > 10:1）。如果未来出现性能问题，可以考虑单独打包超大 session。

## 7. 不变的部分

以下逻辑保持不变：

- **归档层级**：日包 → 周包 → 月包 → 年包
- **触发时机**：服务启动 + 每日定时
- **幂等补偿**：`check_and_archive_all` 的 30 天回溯补偿逻辑
- **锁机制**：flock 互斥
- **压缩参数**：xz -9e -T0 + nice -n 19
- **路径命名**：`{year}/{month}/W{week}/{day}.tar.xz` 等

## 8. 实施步骤

### Step 1: 引入 `ArchivableSession` + `find_archivable_sessions`

- 新增 `ArchivableSession` 结构体
- 实现 `find_archivable_sessions`，递归扫描 session + subagents
- 保留旧 `find_archivable_files` 函数签名（deprecated），内部调用新函数做转换
- 单元测试：构造包含 subagent 的目录结构，验证发现逻辑

### Step 2: 改造 `Compressor`

- 新增 `compress_with_structure(base_dir, relative_paths, output)`
- 删除旧 `compress_files` 的第一段死代码（lines 74-79）
- 修改 `decompress` 支持子目录结构（已支持，无需改动）
- 单元测试：打包→解压→校验目录结构完整

### Step 3: 改造 `do_archive` 和 `do_merge`

- `do_archive`：接收 `Vec<ArchivableSession>`，提取相对路径调用 `compress_with_structure`
- `do_archive` 验证：用相对路径查找解压文件
- `do_merge`：隔离子目录 + 递归收集 + 去重
- 集成测试：模拟完整的日归档→周合并→解压验证流程

### Step 4: 端到端验证

- 在 `~/.claude/projects/` 上运行，验证 subagent 文件被正确归档
- 验证旧归档仍可被正确合并
- 验证还原：`tar -xJf` 后目录结构匹配原始

## 9. 依赖

- 新增 `walkdir` crate（递归目录遍历）
- 其他依赖不变

## 10. 风险

| 风险 | 缓解 |
|------|------|
| 旧归档合并后目录结构混合 | 可接受，旧数据无 subagent。可选：一次性迁移脚本 |
| Session 数量大时扫描慢 | effective_mtime 计算是 O(n) stat 调用，可并行化 |
| `-Users-...` 路径被 tar 解析为选项 | `compress_with_structure` 强制添加 `--` 参数分隔符 |
| 相对路径逃逸（`..`） | `strip_prefix` + 显式检查，拒绝非法路径 |
| 归档包体积增大（多了 subagent） | xz 压缩率高，且 subagent 的 JSONL 更短小 |
| merge 时 `find_weekly_archives` 返回顺序不稳定 | merge 前按路径名排序，保证去重确定性 |
| 归档期间并发写入 | 24h quiet period + SHA-256 验证双重保障，失败自动回滚 |
