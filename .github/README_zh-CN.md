中文 | [English](/README.md)

# ETerm Workspace

ETerm 工具生态系统的 Cargo Workspace 主仓库，用于协调和管理所有子项目。

## 项目结构

```
ETerm-workspace/
├── ai-cli-session-collector/   # JSONL 解析器
├── ai-cli-session-db/          # 数据库层 + FFI
├── claude/                     # Vlaude 服务 (daemon + server)
├── english/                    # ETerm.app (Swift/macOS)
└── memex/                      # Memex 后端
```

## 快速开始

```bash
# 克隆（包含所有子模块）
git clone --recursive git@github.com:vimo-ai/ETerm-workspace.git
cd ETerm-workspace

# 编译所有 Rust 组件
./scripts/build.sh

# 或分别编译
./scripts/build.sh ffi      # FFI 动态库
./scripts/build.sh memex    # Memex 二进制
./scripts/build.sh plugins  # Swift 插件
```

## 子模块管理

```bash
# 拉取/更新所有子模块
git submodule update --init --recursive

# 更新子模块到最新
git submodule update --remote
```

## 文档

- [架构设计](../docs/architecture/overview.md) - 架构设计
- [开发指南](../docs/guides/development.md) - 开发指南

## 仓库列表

| 子项目 | 仓库 | 说明 |
|--------|------|------|
| ai-cli-session-collector | [vimo-ai/AI-cli-session-collector](https://github.com/vimo-ai/AI-cli-session-collector) | JSONL 解析 |
| ai-cli-session-db | [vimo-ai/ai-cli-session-db](https://github.com/vimo-ai/ai-cli-session-db) | 数据库 + FFI |
| claude | [vimo-ai/vlaude](https://github.com/vimo-ai/vlaude) | Vlaude 服务 |
| english | [vimo-ai/ETerm](https://github.com/vimo-ai/ETerm) | macOS 应用 |
| memex | [vimo-ai/memex](https://github.com/vimo-ai/memex) | Memex 后端 |
