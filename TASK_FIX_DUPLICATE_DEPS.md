# 任务：修复 Rust 层重复依赖

## 背景

当前架构存在重复依赖问题：

```
memex-rs
├── ai-cli-session-collector  ← 直接依赖
└── claude-session-db
        └── ai-cli-session-collector  ← 间接依赖（重复！）
```

同时，Swift 插件需要依赖两个 FFI：
- `SessionReaderFFI` (解析)
- `SharedDbFFI` (数据库)

## 目标

让 `claude-session-db` 成为唯一入口点：

```
memex-rs
└── claude-session-db
        └── ai-cli-session-collector (内部依赖)
        └── pub use ... (re-export 公开类型)
        └── FFI 暴露解析函数
```

## 修改步骤

### 第一步：claude-session-db re-export

文件：`/claude-session-db/src/lib.rs`

```rust
// 添加 re-export
pub use ai_cli_session_collector::{
    ClaudeAdapter,
    CodexAdapter,
    ConversationAdapter,
    ParsedMessage,
    SessionMeta,
    IndexableMessage,
    IndexableSession,
    MessageType,
    Source,
};
```

### 第二步：claude-session-db FFI 暴露解析函数

文件：`/claude-session-db/src/ffi.rs`

添加以下 FFI 函数：
- `session_db_parse_jsonl(path) -> CParseResult` - 解析单个 JSONL 文件
- `session_db_list_sessions(projects_path) -> CSessionList` - 列出所有会话
- `session_db_free_parse_result(result)` - 释放解析结果

### 第三步：更新 memex-rs 依赖

文件：`/memex/memex-rs/Cargo.toml`

```toml
[dependencies]
# 移除这行
# ai-cli-session-collector = { git = "..." }

# 保留这行
claude-session-db = { path = "../../claude-session-db", features = ["writer", "reader", "search", "coordination"] }
```

### 第四步：更新 memex-rs 代码

将所有 `use ai_cli_session_collector::...` 替换为 `use claude_session_db::...`

涉及文件：
- `/memex/memex-rs/src/collector/mod.rs`
- `/memex/memex-rs/src/adapter/registry.rs`
- `/memex/memex-rs/src/adapter/mod.rs`
- `/memex/memex-rs/src/ffi.rs`
- `/memex/memex-rs/src/db/mod.rs`

### 第五步：更新 Swift 插件

移除 `SessionReaderFFI` 依赖，改用 `SharedDbFFI` 的解析函数。

涉及文件：
- `/english/Plugins/VlaudeKit/Package.swift` - 移除 SessionReaderFFI target
- `/english/Plugins/VlaudeKit/Sources/VlaudeKit/SessionReader.swift` - 改用 SharedDbFFI
- `/english/Plugins/MemexKit/Package.swift` - 同上
- `/english/Plugins/MemexKit/Sources/MemexKit/SessionReader.swift` - 同上

## 验证

1. `cd claude-session-db && cargo build --features "ffi"`
2. `cd memex/memex-rs && cargo build`
3. `cd english/Plugins/VlaudeKit && ./build.sh`
4. `cd english/Plugins/MemexKit && ./build.sh`
5. 运行 ETerm 测试插件功能

## 注意事项

- 保持 API 兼容性，Swift 侧调用方式尽量不变
- FFI 函数命名统一使用 `session_db_` 前缀
- 内存管理：提供对应的 `free` 函数
