# ETerm 分发计划

> 最后更新: 2026-01-06

## 1. 目标

让 ETerm 可以脱离 `build.sh` 实现分发，用户下载 DMG 后即可使用。

## 2. 当前状态

### 2.1 开发流程

```
开发者:
1. ./scripts/build.sh          # 编译 Rust FFI + memex binary
2. Xcode Build                  # 编译 Swift app + 插件
3. 运行 ETerm.app
```

**问题**: 用户没有 build.sh，无法获得 Rust 编译产物。

### 2.2 组件大小分析

| 组件 | Release 大小 | 说明 |
|------|-------------|------|
| **ETerm.app 核心** | ~50M | 终端 + sugarloaf-ffi |
| **纯 Swift 插件 x9** | ~5M | ClaudeKit, HistoryKit 等 |
| **VlaudeKit** | ~10M | Swift + SharedDB + SocketClient |
| **MemexKit** | ~123M | Swift + SharedDB + memex binary |

### 2.3 插件分类

**纯 Swift 插件（可内置）**:
- ClaudeKit (~290K)
- ClaudeMonitorKit
- DevHelperKit
- HistoryKit (~593K)
- MCPRouterKit
- OneLineCommandKit
- TranslationKit (~904K)
- WorkspaceKit
- WritingKit

**有 Native 依赖的插件（需按需下载）**:
- VlaudeKit: libclaude_session_db.dylib (2.6M) + libsocket_client_ffi.dylib (6.1M)
- MemexKit: libclaude_session_db.dylib (2.6M) + memex binary (119M)

### 2.4 共享依赖

| 依赖 | 大小 | 被谁使用 |
|------|------|---------|
| libclaude_session_db.dylib | 2.6M | VlaudeKit, MemexKit |
| libsocket_client_ffi.dylib | 6.1M | VlaudeKit |
| memex binary | 119M | MemexKit, CLI 用户 |

---

## 3. 分发架构设计

### 3.1 目录结构

```
/Applications/ETerm.app/
├── Contents/
│   ├── MacOS/ETerm
│   ├── Frameworks/
│   │   └── libsugarloaf_ffi.dylib      # 终端渲染（必须内置）
│   ├── PlugIns/                         # 内置插件
│   │   ├── ClaudeKit.bundle
│   │   ├── HistoryKit.bundle
│   │   └── ... (其他纯 Swift 插件)
│   └── Resources/

~/.vimo/                                  # 共享运行时
├── lib/                                  # [可重装] 动态库
│   ├── libclaude_session_db.dylib
│   └── libsocket_client_ffi.dylib
├── bin/                                  # [可重装] 二进制
│   └── memex
├── plugins/                              # [可重装] 下载的插件
│   ├── VlaudeKit.bundle/
│   └── MemexKit.bundle/
├── db/                                   # [用户数据 - 永不删除!]
│   └── claude-session.db
└── version.json                          # [可重装] 版本清单
```

### 3.2 version.json 格式

```json
{
  "runtime_version": "0.0.1-beta.1",
  "components": {
    "libclaude_session_db": {
      "version": "0.0.1-beta.1",
      "sha256": "abc123...",
      "installed_by": "VlaudeKit",
      "installed_at": "2025-01-05T12:00:00Z"
    },
    "memex": {
      "version": "0.0.1-beta.1",
      "sha256": "def456...",
      "installed_by": "MemexKit"
    }
  },
  "plugins": {
    "VlaudeKit": {
      "version": "0.0.1-beta.1",
      "installed_at": "2025-01-05T12:00:00Z"
    },
    "MemexKit": {
      "version": "0.0.1-beta.1",
      "installed_at": "2025-01-05T12:00:00Z"
    }
  }
}
```

### 3.3 插件 manifest.json 格式

在现有 SDK 插件 manifest 基础上扩展 `distribution` 字段：

```json
{
  "id": "com.eterm.memex",
  "name": "Memex",
  "version": "0.0.1-beta.1",
  "minHostVersion": "0.0.1-beta.1",
  "sdkVersion": "0.0.1-beta.1",
  "runMode": "main",
  "dependencies": [],
  "capabilities": ["ui.sidebar"],
  "principalClass": "MemexPlugin",
  "sidebarTabs": [...],

  "distribution": {
    "description": "Claude 会话历史搜索",
    "size": "<auto>",
    "sha256": "<auto>",
    "runtime_deps": [
      {
        "name": "libclaude_session_db",
        "min_version": "0.0.1-beta.1",
        "path": "lib/libclaude_session_db.dylib",
        "sha256": "<auto>",
        "download_url": "<auto>"
      },
      {
        "name": "memex",
        "min_version": "0.0.1-beta.1",
        "path": "bin/memex",
        "sha256": "<auto>",
        "download_url": "<auto>"
      }
    ]
  }
}
```

**说明**：
- 纯 Swift 插件没有 `distribution` 字段
- VlaudeKit/MemexKit 有 `distribution` 字段，声明 native 依赖
- `size`、`sha256` 由 CI 打包脚本自动计算填入，不手动维护
- 开发时 manifest 只需声明 `runtime_deps` 的 `name`、`min_version`、`path`
- CI 打包时自动补全 `size`、`sha256`、`download_url`

---

## 4. 实施计划

### Phase 1: 打包基础

**目标**: 让 ETerm.app 能脱离 build.sh 独立运行

| 步骤 | 任务 | 详细说明 |
|------|------|---------|
| 1.1 | Xcode 嵌入 sugarloaf-ffi | Build Phases → Embed Frameworks |
| 1.2 | 内置纯 Swift 插件 | 9 个插件编译进 PlugIns/ 目录 |
| 1.3 | 修正 dylib rpath | install_name_tool 确保路径正确 |
| 1.4 | 打包脚本 | scripts/package.sh → 签名 + DMG |

**验收标准**:
- [x] 新机器下载 DMG，拖拽安装后终端功能正常
- [x] 内置插件自动加载
- [x] 无 build.sh 依赖

**已完成**：
- `PluginLoader.swift` 支持从 app 内置 `PlugIns/` 加载
- `scripts/build-plugins.sh` 编译 9 个纯 Swift 插件
- `scripts/package.sh` 打包签名 DMG

---

### Phase 2: 插件下载系统

**目标**: 用户可以下载安装 VlaudeKit/MemexKit

| 步骤 | 任务 | 详细说明 |
|------|------|---------|
| 2.1 | 定义 manifest 规范 | 插件元数据 + 依赖声明 |
| 2.2 | Runtime Deps CI | 各 repo 发布 dylib/binary |
| 2.3 | ETerm Release CI | 发布 DMG + 可下载插件 bundle |
| 2.4 | PluginDownloader | 下载、校验、解压、安装 |
| 2.5 | 共享依赖管理 | 检测已安装组件，避免重复下载 |
| 2.6 | 插件加载器更新 | 支持从 ~/.vimo/plugins/ 加载 |

**CI 架构**：

```
ETerm Release (vX.X.X) 发布：
├── ETerm.dmg                    # 主程序 + 纯 Swift 插件
├── VlaudeKit.bundle.zip         # 可下载插件 bundle
└── MemexKit.bundle.zip          # 可下载插件 bundle

Runtime Deps (各 repo 独立发布)：
├── ai-cli-session-db → libclaude_session_db.dylib  ✅ 已完成
├── vlaude → libsocket_client_ffi.dylib             ✅ 已完成
└── memex → memex binary                            ✅ 已完成
```

**PluginDownloader 下载流程**：
1. 从 ETerm release 下载 VlaudeKit.bundle.zip / MemexKit.bundle.zip
2. 根据 manifest.runtime_deps 从各 repo release 下载 dylib/binary
3. 解压安装到 ~/.vimo/

**验收标准**:
- [ ] 设置 → 插件 → 可以下载安装 VlaudeKit
- [ ] 设置 → 插件 → 可以下载安装 MemexKit
- [ ] 先装 VlaudeKit 再装 MemexKit，SharedDB 不重复下载
- [ ] 插件可卸载

**已完成代码**：
- `Distribution/DistributionManifest.swift` - 分发数据模型
- `Distribution/VersionManager.swift` - 管理 ~/.vimo/version.json
- `Distribution/PluginDownloader.swift` - 下载、校验、原子安装
- `Distribution/DownloadablePluginView.swift` - 可下载插件 UI
- `PluginManagerView.swift` - 集成可下载插件区域

**已完成 CI**：
- [x] ai-cli-session-db release CI (libclaude_session_db.dylib)
- [x] vlaude release CI (libsocket_client_ffi.dylib)
- [x] memex release CI (memex binary)

**待完成**：
- ETerm release CI (DMG + 插件 bundle)
- 实际测试下载安装流程

---

### Phase 3: 首次启动向导

**目标**: 完整的首次使用体验

| 步骤 | 任务 | 详细说明 |
|------|------|---------|
| 3.1 | FirstLaunchManager | 检测是否首次启动 |
| 3.2 | OnboardingView | 欢迎页 → 功能选择 → 下载 → 完成 |
| 3.3 | 下载进度 UI | 进度条、速度、剩余时间 |
| 3.4 | 错误处理 | 网络失败重试、断点续传 |

**验收标准**:
- [ ] 首次启动显示向导
- [ ] 可选择安装 Memex/Vlaude
- [ ] 下载过程有进度显示
- [ ] 可跳过，稍后在设置中安装

**向导 UI 草图**:

```
┌─────────────────────────────────────────┐
│           欢迎使用 ETerm                 │
│                                         │
│  以下功能需要额外下载：                   │
│                                         │
│  ☐ Memex 历史搜索 (123 MB)              │
│    └ 搜索所有 Claude 对话历史            │
│                                         │
│  ☐ Vlaude 远程控制 (10 MB)              │
│    └ 从 iOS 查看和控制 Claude            │
│                                         │
│  ┌───────────────────────────────────┐  │
│  │ 稍后可在 设置 → 插件 中安装         │  │
│  └───────────────────────────────────┘  │
│                                         │
│          [跳过]      [安装选中]          │
└─────────────────────────────────────────┘
```

---

### Phase 4: 插件市场（后续）

**目标**: 完整的插件生态

| 步骤 | 任务 |
|------|------|
| 4.1 | 插件列表 API | 从 GitHub 获取可用插件列表 |
| 4.2 | 插件市场 UI | 发现、搜索、分类 |
| 4.3 | 自动更新检测 | 启动时检查插件更新 |
| 4.4 | 第三方插件支持 | 插件开发者文档 |

---

## 5. 安装场景处理

### 场景 A: 先装 memex CLI，再装 MemexKit

```
1. brew install memex
   → ~/.vimo/bin/memex
   → ~/.vimo/lib/libclaude_session_db.dylib
   → version.json 记录

2. ETerm 安装 MemexKit
   → 检测 ~/.vimo/bin/memex 已存在
   → 比较版本
   → 版本兼容 → 跳过 memex 下载
   → 只下载 MemexKit.bundle (~1M)
```

### 场景 B: 先装 MemexKit，再装 memex CLI

```
1. ETerm 安装 MemexKit
   → ~/.vimo/bin/memex
   → ~/.vimo/lib/libclaude_session_db.dylib
   → ~/.vimo/plugins/MemexKit.bundle
   → version.json 记录

2. brew install memex
   → 检测 ~/.vimo/bin/memex 已存在
   → 版本相同 → 跳过，只创建 symlink
```

### 场景 C: 装 VlaudeKit + MemexKit

```
1. 安装 VlaudeKit
   → libclaude_session_db.dylib ✅
   → libsocket_client_ffi.dylib ✅
   → VlaudeKit.bundle ✅

2. 安装 MemexKit
   → libclaude_session_db.dylib 已存在，跳过
   → memex binary ✅
   → MemexKit.bundle ✅
```

---

## 6. 平台要求

| 要求 | 说明 |
|------|------|
| **架构** | Apple Silicon (M1/M2/M3/M4) 仅 arm64 |
| **系统** | macOS 13.0 (Ventura) 及以上 |
| **Intel** | 暂不支持，后续视需求决定 |

---

## 7. 签名与公证流程

### 7.1 签名顺序

```bash
# 1. 签名所有 dylib（从内到外）
codesign --force --sign "Developer ID Application: XXX" \
  ETerm.app/Contents/Frameworks/*.dylib

# 2. 签名 app
codesign --force --sign "Developer ID Application: XXX" \
  --options runtime \
  --entitlements ETerm.entitlements \
  ETerm.app

# 3. 验证
codesign --verify --deep --strict ETerm.app
```

### 7.2 公证

```bash
# 1. 打包成 zip
ditto -c -k --keepParent ETerm.app ETerm.zip

# 2. 提交公证
xcrun notarytool submit ETerm.zip \
  --apple-id "xxx@xxx.com" \
  --team-id "XXXXXXXXXX" \
  --password "@keychain:AC_PASSWORD" \
  --wait

# 3. Staple
xcrun stapler staple ETerm.app

# 4. 打包 DMG
hdiutil create -volname "ETerm" -srcfolder ETerm.app -ov ETerm.dmg
```

### 7.3 插件签名

从 GitHub Release 下载的插件/dylib 也需要签名：
- 方案 A：CI 打包时签名（推荐）
- 方案 B：下载后本地 ad-hoc 签名（需要 Hardened Runtime 例外）

---

## 8. 版本检查策略（简化版）

```
插件安装时：
1. 检查 min_eterm_version，不满足 → 提示升级 ETerm
2. 检查 runtime_deps 的 min_version
   - 已安装且版本够 → 跳过下载
   - 已安装但版本低 → 提示升级（覆盖安装）
   - 未安装 → 下载安装
```

**不做的事**（MVP 阶段）：
- ABI 兼容检查
- 灰度发布

### 8.1 原子安装流程

安装/更新必须是原子性的，要么成功，要么保持原状：

```
1. 下载到临时目录 (~/.vimo/tmp/xxx.downloading)
2. 校验 sha256
3. 校验通过 → rename 到目标位置（原子操作）
4. 清理临时文件
```

**失败场景**：
- 下载中断 → 临时文件删除，原文件不动
- 校验失败 → 临时文件删除，原文件不动
- rename 失败 → 原文件还在，提示重试

**不存在需要用户手动删除的情况**。

---

## 9. 错误处理（简化版）

| 场景 | 处理 |
|------|------|
| 下载失败 | 提示重试，最多 3 次 |
| 校验失败 (sha256) | 删除文件，提示重新下载 |
| 签名验证失败 | 提示用户检查来源 |
| 版本不兼容 | 提示升级 ETerm 或插件 |

---

## 10. 风险和注意事项

### 10.1 dylib 路径

- 使用 `@rpath` 或 `@loader_path` 确保可移植
- 打包后用 `otool -L` 验证路径正确

### 10.2 网络问题

- GitHub Release 在国内可能慢
- MVP 阶段暂不做镜像，后续按需添加

### 10.3 数据目录

- `~/.vimo/db/` 数据不随插件卸载删除
- 完全清理需要用户手动删除 `~/.vimo/`

---

## 11. 时间线

| Phase | 预计工作量 | 依赖 |
|-------|-----------|------|
| Phase 1 | 2-3 天 | 无 |
| Phase 2 | 3-5 天 | Phase 1 |
| Phase 3 | 2-3 天 | Phase 2 |
| Phase 4 | 持续迭代 | Phase 3 |

**MVP**: Phase 1 + Phase 2 完成即可分发
