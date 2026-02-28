# ETerm 插件系统架构

> 本文档记录插件系统的完整架构，包括构建机制、依赖关系、CI 流程等。

## 插件分类

ETerm 插件分为两类，**构建方式完全不同**：

| 类型 | 插件 | 构建方式 | ETermKit 依赖 |
|------|------|---------|--------------|
| **内置插件** | AICliKit, ClaudeKit, ClaudeMonitorKit, HistoryKit, OneLineCommandKit, TranslationKit, WorkspaceKit, WritingKit | Xcode + `build_all_plugins.sh` | 预编译 framework |
| **外部插件** | VlaudeKit, MemexKit, MCPRouterKit | 独立 `swift build` | 预编译 framework |

### 为什么分开？

- **内置插件**：随 ETerm.app 一起发布，嵌入 `ETerm.app/Contents/PlugIns/`
- **外部插件**：有原生依赖（Rust FFI），需要单独下载安装到 `~/.vimo/eterm/plugins/`

---

## ETermKit SDK

ETermKit 是所有插件的公共依赖，提供插件 API。

### 构建命令

```bash
./scripts/build.sh etermkit
```

### 产出物

```
ETerm/Build/
├── ETermKit.framework/
│   ├── Versions/
│   │   └── A/
│   │       ├── ETermKit                 # dylib
│   │       ├── Modules/
│   │       │   └── ETermKit.swiftmodule/
│   │       │       ├── arm64-apple-macosx.swiftmodule
│   │       │       ├── arm64-apple-macosx.swiftdoc
│   │       │       ├── arm64-apple-macosx.abi.json
│   │       │       ├── arm64-apple-macos.swiftmodule   # 兼容不同工具链
│   │       │       ├── arm64-apple-macos.swiftdoc
│   │       │       └── arm64-apple-macos.abi.json
│   │       └── Resources/
│   │           └── Info.plist
│   ├── ETermKit -> Versions/Current/ETermKit
│   ├── Modules -> Versions/Current/Modules
│   └── Resources -> Versions/Current/Resources
└── ETermKit.swiftmodule/                # 兼容旧方式
    └── ...
```

### 关键细节：swiftmodule triple 命名

Swift 工具链对 triple 命名不一致：
- Swift PM: `arm64-apple-macos`
- Xcode: `arm64-apple-macosx`

**必须同时提供两种命名**，否则会报错：
```
could not find module 'ETermKit' for target 'arm64-apple-macos'; found: arm64-apple-macosx
```

---

## 构建流程

### 本地开发

```bash
# 1. 先构建 ETermKit（必须！）
./scripts/build.sh etermkit

# 2. 构建所有 Rust FFI
./scripts/build.sh ffi          # claude-session-db
./scripts/build.sh socket       # socket-client-ffi
./scripts/build.sh mcp-router   # mcp-router-core

# 3. 构建插件
./scripts/build.sh plugins      # 外部插件 (VlaudeKit, MemexKit)

# 或者一次性全部构建
./scripts/build.sh
```

### 内置插件构建（Xcode）

内置插件通过 Xcode 构建，不能独立 `swift build`：

```bash
# 由 Xcode build phase 或脚本调用
./scripts/build_all_plugins.sh
```

脚本会：
1. 扫描 `Plugins/` 下所有 `*Kit` 目录
2. 调用每个插件的 `build.sh`
3. 输出到 `$BUNDLE_OUTPUT_DIR`（由 Xcode 设置）

### 外部插件构建

外部插件可以独立构建：

```bash
cd ETerm/Plugins/VlaudeKit
swift build -c release
```

但必须先确保：
1. ETermKit.framework 已构建（`./scripts/build.sh etermkit`）
2. 原生依赖已就位（`Libs/SharedDB/`, `Libs/SocketClient/` 等）

---

## CI 发布流程

### ETerm 主应用 (`release-eterm.yml`)

触发：`git tag eterm-v*`

流程：
1. 构建 sugarloaf-ffi (Rust)
2. 构建 ETermKit.framework
3. 构建内置插件
4. 构建 ETerm.app
5. 嵌入插件到 app bundle
6. 签名、打包 DMG
7. 创建 GitHub Release

### 外部插件 (`release-plugins.yml`)

触发：`git tag vlaudekit-v*`, `memexkit-v*`, `mcprouterkit-v*`

流程：
1. 从 `plugin-deps.json` 读取依赖配置
2. 下载预编译的原生依赖（从其他 repo 的 release）
3. 构建 ETermKit.framework（**重要：当前代码需要这步**）
4. `swift build -c release`
5. 打包 bundle
6. 创建 GitHub Release

### 依赖配置 (`.github/plugin-deps.json`)

```json
{
  "vlaudekit": {
    "name": "VlaudeKit",
    "path": "Plugins/VlaudeKit",
    "deps": {
      "libclaude_session_db": {
        "repo": "vimo-ai/ai-cli-session-db",
        "tag": "v0.0.1-beta.1",
        "asset": "libclaude_session_db.dylib",
        "dest": "Libs/SharedDB/"
      }
    }
  }
}
```

---

## 历史演变

### 2026-01-15: 插件改用预编译 framework

**变更**：`61cb8ff 🔧 Migrate plugins to use prebuilt ETermKit framework`

**之前**：
```swift
// Package.swift
dependencies: [
    .package(path: "../../Packages/ETermKit"),
]
```
SwiftPM 从源码编译 ETermKit。

**之后**：
```swift
// Package.swift
let etermkitPath = "../../Build"
swiftSettings: [
    .unsafeFlags(["-F", etermkitPath])
]
```
需要预编译的 ETermKit.framework。

**影响**：
- 本地开发必须先 `./scripts/build.sh etermkit`
- CI 必须加入 ETermKit 构建步骤
- 旧 tag（如 `vlaudekit-v0.0.1-beta.1`）仍用源码依赖，所以 CI 还能工作
- **新 tag 发布前必须更新 `release-plugins.yml`**

---

## 常见问题

### Q: `no such module 'ETermKit'`

**原因**：ETermKit.framework 未构建或结构不对

**解决**：
```bash
./scripts/build.sh etermkit
```

### Q: `could not find module 'ETermKit' for target 'arm64-apple-macos'`

**原因**：swiftmodule 只有 `macosx` 命名，缺少 `macos` 命名

**解决**：确保 `scripts/build.sh` 同时生成两种 triple 命名

### Q: 外部插件 CI 失败

**检查**：
1. `release-plugins.yml` 是否有 ETermKit 构建步骤
2. `plugin-deps.json` 依赖版本是否正确
3. 依赖的 release 是否存在

---

## 关键文件

| 文件 | 作用 |
|------|------|
| `scripts/build.sh` | 统一构建入口 |
| `scripts/build_all_plugins.sh` | 内置插件批量构建 |
| `ETerm/Build/ETermKit.framework/` | SDK framework |
| `ETerm/Packages/ETermKit/` | SDK 源码 |
| `ETerm/Plugins/*/Package.swift` | 插件包定义 |
| `.github/workflows/release-eterm.yml` | 主应用 CI |
| `.github/workflows/release-plugins.yml` | 外部插件 CI |
| `.github/plugin-deps.json` | 外部插件依赖配置 |
