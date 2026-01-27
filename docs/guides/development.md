# ETerm 开发指南

## 项目架构

ETerm 采用 Git Submodule + Cargo Workspace 架构，子项目独立维护，主仓库统一编译。

```
ETerm/                              # 主仓库 (workspace)
├── Cargo.toml                      # Cargo workspace + [patch]
├── scripts/build.sh                # 统一编译入口
│
├── ai-cli-session-collector/       # submodule: JSONL 解析器
├── ai-cli-session-db/              # submodule: 数据库 + FTS + FFI
├── memex/memex-rs/                 # submodule: Memex 后端
├── claude/packages/vlaude-core/    # 独立 workspace: daemon 相关
│
└── english/                        # ETerm 主项目 (Swift)
    └── Plugins/
        ├── VlaudeKit/              # 使用 ai-cli-session-db FFI
        └── MemexKit/               # 使用 ai-cli-session-db FFI + memex binary
```

## 依赖关系

```
ai-cli-session-collector (JSONL 解析)
        │
        ↓ git 依赖
ai-cli-session-db (数据库 + FTS + Writer 协调 + FFI)
        │
        ├──────────────────┬──────────────────┐
        ↓                  ↓                  ↓
session-reader         memex-rs           VlaudeKit
(vlaude-core)                             MemexKit
        │                  │              (Swift FFI)
        ↓                  ↓
daemon-logic           MemexKit
```

### 依赖策略

- **子项目使用 git 依赖**：支持独立 clone、独立编译
- **主仓库使用 [patch]**：开发时自动使用本地版本

```toml
# ETerm/Cargo.toml
[patch."https://github.com/vimo-ai/ai-cli-session-collector"]
ai-cli-session-collector = { path = "ai-cli-session-collector" }
```

## 开发流程

### 1. 克隆项目

```bash
git clone --recursive https://github.com/vimo-ai/ETerm
cd ETerm
```

### 2. 编译

```bash
# 编译所有 Rust 组件并部署到插件目录
./scripts/build.sh

# 或分别编译
./scripts/build.sh ffi      # 只编译 ai-cli-session-db FFI
./scripts/build.sh memex    # 只编译 memex binary
./scripts/build.sh plugins  # 只构建 Swift 插件
```

### 3. 修改后更新下游

| 修改的项目 | 运行命令 |
|-----------|---------|
| ai-cli-session-collector | `cd ai-cli-session-collector && ./scripts/update_downstream.sh` |
| ai-cli-session-db | `cd ai-cli-session-db && ./scripts/update_downstream.sh` |
| memex-rs | `cd memex/memex-rs && ./scripts/update_eterm.sh` |
| vlaude-core | `cargo build` (在 vlaude-core 目录) |

### 4. 构建 Swift 插件

```bash
cd english/Plugins/VlaudeKit && ./build.sh
cd english/Plugins/MemexKit && ./build.sh
```

## 目录说明

| 目录 | 说明 | 独立仓库 |
|------|------|---------|
| ai-cli-session-collector | Claude/Codex JSONL 解析器 | ✅ |
| ai-cli-session-db | SQLite 数据库 + FTS5 + Writer 协调 | ✅ |
| memex/memex-rs | Memex 后端 (HTTP + 向量搜索) | ✅ |
| claude/packages/vlaude-core | Vlaude daemon 核心 | ✅ |
| english | ETerm 主项目 (Swift/macOS) | - |

## 关键文件

| 文件 | 作用 |
|------|------|
| ETerm/Cargo.toml | Workspace 定义 + [patch] 本地依赖 |
| scripts/build.sh | 统一编译入口 |
| */scripts/update_downstream.sh | 更新下游项目 |
| english/Plugins/*/build.sh | Swift 插件构建 |

## 注意事项

1. **不要硬编码路径**：所有脚本使用相对路径推断
2. **谁修改谁更新**：修改后运行对应的 `update_downstream.sh`
3. **子项目独立性**：子项目使用 git 依赖，可独立 clone 编译
4. **FFI 同步**：修改 ai-cli-session-db 后需要重新复制 dylib 到插件目录

## 常见问题

### Q: cargo build 报错找不到依赖？

确保在 ETerm 根目录运行，`[patch]` 才会生效：
```bash
cd /path/to/ETerm
cargo build
```

### Q: 插件加载失败？

检查 FFI dylib 是否已复制到插件目录：
```bash
ls english/Plugins/VlaudeKit/Libs/SharedDB/
# 应该有 libai_cli_session_db.dylib
```

### Q: 独立编译子项目？

子项目可以独立编译（使用 git 依赖）：
```bash
cd ai-cli-session-db
cargo build  # 会从 git 拉取 ai-cli-session-collector
```
