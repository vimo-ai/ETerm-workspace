# DevRunner 设计文档

> 通用项目启动器，支持 Xcode / Node 等多种项目类型

## 概述

DevRunner 是一个独立的 macOS 应用，同时提供 ETerm 插件集成。用于管理和启动开发项目，替代频繁打开 Xcode 等重量级 IDE。

### 核心场景

- 启动项目（构建 + 运行）
- 查看日志
- 监控 CPU / 内存

### 设计原则

- **独立优先**：独立 App 功能完整，不依赖 ETerm
- **ETerm 增强**：在 ETerm 中使用时，利用终端能力获得更好体验
- **Provider 扩展**：通过 Provider 模式支持多种项目类型

---

## 支持的项目类型

| 类型 | 检测标识 | 构建工具 | 运行方式 |
|------|----------|----------|----------|
| Xcode | `*.xcodeproj` | xcodebuild | simctl / devicectl |
| Node | `package.json` | npm / pnpm | node / npm run |
| (未来) Rust | `Cargo.toml` | cargo | cargo run |
| (未来) Python | `pyproject.toml` | pip | python |

---

## 功能模块

### 1. 项目扫描

**独立 App**：
- 用户手动添加项目路径
- 不做自动扫描

**ETerm 插件**：
- 跟随 WorkspaceKit 的 workspace
- 自动检测 workspace 下的项目

**Xcode 项目解析**：
- 只扫描 `.xcodeproj`（暂不支持 `.xcworkspace`）
- 解析 schemes 列表（供用户选择）
- 解析 bundle id（用于日志过滤）

**Node 项目解析**：
- 检测 `package.json`
- 解析 scripts（npm run xxx）
- 检测包管理器（npm / pnpm / yarn）

---

### 2. 设备管理（Xcode）

**设备类型**：
- macOS：本机，固定
- iOS 模拟器：`xcrun simctl list -j`
- iOS 真机：`xcrun devicectl list devices -j`（USB + Wi-Fi 配对）

**展示方式**：
- 全部显示
- 按 iOS 版本归类
- 标记运行状态（Running / Shutdown）

**模拟器启动**：
- 选择未启动的模拟器时，Run 时自动执行 `simctl boot`

**真机**：
- 签名依赖 Xcode 已有配置
- 支持 Wi-Fi 调试（需先在 Xcode 中配对）

---

### 3. 构建运行

**Xcode**：
```
xcodebuild → simctl install → simctl launch
```

**Node**：
```
npm install (if needed) → npm run xxx
```

**配置**：
| 配置项 | 方案 |
|--------|------|
| 构建配置 | 默认 Debug |
| 构建模式 | 默认增量，可选 Clean |
| 输出格式 | 默认格式化（xcbeautify），原始日志可选查看 |

**构建失败**：
- 错误信息在输出中显示
- 格式化工具保留并高亮错误

---

### 4. 日志流

**获取方式**：
```bash
# iOS 模拟器
xcrun simctl spawn booted log stream --predicate 'subsystem == "com.xxx"'

# iOS 真机
xcrun devicectl device syslog --device <udid>

# macOS
log stream --predicate 'subsystem == "com.xxx"'

# Node
直接捕获 stdout / stderr
```

**功能**：
- 默认按 bundle id / 进程过滤
- 可筛选日志级别（debug / info / warning / error）
- 实时流 + 可暂停 + 可搜索
- 一次看一个设备/进程

---

### 5. 实时监控

**指标**：
- CPU 使用率
- 内存占用

**实现状态**：
- ✅ Rust 层 `ProcessMonitor` 已实现 (sysinfo)
- ⏸️ UI 展示暂未集成（终端方案下进程由 PTY 管理，PID 获取复杂）

**原设计展示方案**（待后续实现）：
```
┌─ 项目列表 ──────────────────────────────────┐
│ ● MyApp                    CPU 12% | 45MB  │
│ ● node-server              CPU 3%  | 128MB │
│ ○ SDK-Demo                                 │
└─────────────────────────────────────────────┘
```

---

### 6. Profile（Xcode）

- 深度 Profile：不做，用户自己开 Instruments
- 实时监控：见上一节

---

## 独立 App UI

**窗口形态**：
- 常规窗口
- 不做菜单栏 App

**布局**：
- 左右 master-detail 布局
- 左侧：项目列表
- 右侧：项目详情 + 控制面板 + 日志

```
┌─ DevRunner ────────────────────────────────────────────────┐
│ Projects               │  MyApp                           │
│ ───────────────────    │  ─────────────────────────────── │
│ ● MyApp     12% 45MB   │  Scheme: [MyApp ▼]               │
│ ● node-srv   3% 128MB  │  Device: [iPhone 15 Pro ▼]       │
│ ○ SDK-Demo             │  Config: [Debug ▼]               │
│                        │  ──────────────────────────────  │
│ ───────────────────    │  [▶ Run] [🔧 Build] [■ Stop]     │
│ [+ Add]                │  ──────────────────────────────  │
│                        │  📋 Logs                         │
│                        │  ┌────────────────────────────┐  │
│                        │  │ 10:23:01 App launched      │  │
│                        │  │ 10:23:02 Loading...        │  │
│                        │  └────────────────────────────┘  │
└────────────────────────┴───────────────────────────────────┘
```

---

## ETerm 集成（计划中）

> **状态**：暂未实现，当前聚焦独立 App

**预期方案**（与独立 App 类似）：
- 作为 ETerm 插件 (`DevRunnerKit`)
- 只依赖 `dev-runner-core`
- 用 ETerm 终端执行命令（与独立 App 方案一致）

```
┌─ ETerm ────────────────────────────────────────────────────┐
│ [🔨 DevRunner] [Build: MyApp] [Logs: MyApp]  ← 多个 Tab    │
├────────────────────────────────────────────────────────────┤
│  DevRunner Tab: 控制面板 (Scheme/Device 选择)              │
│  Build Tab: xcodebuild 输出（ETerm 终端）                  │
│  Logs Tab: log stream 输出（ETerm 终端）                   │
└────────────────────────────────────────────────────────────┘
```

**Workspace 联动**：
- 检测当前 workspace 下的项目
- 自动识别项目类型

---

## 项目结构

### Rust 分层架构

```
┌─────────────────────────────────────────────────────────────────┐
│                      Swift 层                                    │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                   独立 App (DevRunner.app)               │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │   │
│  │  │ ContentView │  │ SidebarView │  │ MultiTerminalView│  │   │
│  │  └──────┬──────┘  └─────────────┘  └────────┬────────┘  │   │
│  │         │                                    │           │   │
│  │         │ FFI (Command 生成)                 │ PTY       │   │
│  │         ▼                                    ▼           │   │
│  │  ┌─────────────┐                    ┌─────────────────┐  │   │
│  │  │ DevRunner   │ ──── Command ────→ │  Terminal执行   │  │   │
│  │  │ (FFI Bridge)│                    │  (直接PTY输出)  │  │   │
│  │  └─────────────┘                    └─────────────────┘  │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
                              │ FFI
┌─────────────────────────────┼───────────────────────────────────┐
│                             ▼                                    │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                  dev-runner-core                         │    │
│  │           (项目检测、Command 生成、设备列表)              │    │
│  └─────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                  dev-runner-app (备用)                   │    │
│  │           (ProcessManager - 当前未使用)                  │    │
│  └─────────────────────────────────────────────────────────┘    │
│                          Rust 层                                 │
└──────────────────────────────────────────────────────────────────┘
```

> **实际方案变更**：Swift App 使用内嵌终端 (PTY) 直接执行命令，
> Rust 层只负责生成 Command。ProcessManager 保留但当前未使用。

### 职责划分

| 层 | crate | 职责 | 当前状态 |
|----|-------|------|----------|
| **core** | `dev-runner-core` | 项目检测、Command 生成、设备列表、配置管理 | ✅ 使用中 |
| **app** | `dev-runner-app` | 进程生命周期、输出捕获、CPU/内存监控 | ⏸️ 保留未使用 |

### 独立 App 实际方案

Swift App **只依赖 core**：
- 用 core 检测项目、生成 Command
- 用内嵌终端 (PTY) 执行命令
- 输出直接走终端渲染，无需 Rust 层捕获
- 这与原计划的 ETerm 插件方案一致

### 目录结构（实际）

```
ETerm/dev-runner/
├── core/                           # 底层 Rust ✅ 使用中
│   ├── Cargo.toml
│   ├── build.rs                    # cbindgen 生成头文件
│   └── src/
│       ├── lib.rs
│       ├── ffi/mod.rs              # FFI 导出
│       ├── adapter/
│       │   ├── traits.rs           # RunnerAdapter trait
│       │   ├── xcode/mod.rs        # XcodeAdapter (883行)
│       │   ├── xcode/devices.rs    # simctl/devicectl
│       │   └── node/mod.rs         # NodeAdapter
│       ├── output/mod.rs           # OutputEvent
│       └── config/mod.rs           # ConfigManager
│
├── app/                            # App 专用层 ⏸️ 保留未使用
│   └── src/
│       ├── process/manager.rs      # ProcessManager
│       ├── process/monitor.rs      # ProcessMonitor
│       └── ffi/                    # FFI 导出
│
├── swift-app/                      # 独立 macOS App ✅
│   ├── DevRunner.xcodeproj
│   └── Sources/
│       ├── App/
│       │   ├── ContentView.swift   # 主界面 (HSplitView)
│       │   ├── SidebarView.swift   # 侧边栏 (树形项目列表)
│       │   └── Theme.swift
│       ├── Terminal/               # 内嵌终端组件
│       │   ├── MultiTerminalView.swift
│       │   └── TerminalTabManager.swift
│       └── FFI/DevRunner.swift     # Rust 桥接
│
├── DESIGN.md
└── TODO.md
```

> ETerm 插件 (`DevRunnerKit`) 暂未创建。

---

## Runner 通用架构

### 实际分层

```
┌─ Swift App ────────────────────────────────────────────────┐
│                                                            │
│  ContentView                    MultiTerminalView          │
│  ├─ Build/Run/Stop 按钮         ├─ PTY 终端渲染            │
│  ├─ Scheme/Device 选择          ├─ 命令执行                │
│  └─ 调用 FFI 生成 Command       └─ 输出直接显示            │
│                                                            │
└────────────────────────────────────────────────────────────┘
                              │ FFI
                              ▼
┌─ dev-runner-core ─────────────────────────────────────────┐
│                                                            │
│  RunnerAdapter (trait)                                     │
│  ├─ detect(path)            # 检测项目类型                 │
│  ├─ targets()               # 可运行目标列表               │
│  ├─ build_cmd()             # 生成构建命令                 │
│  ├─ run_cmd()               # 生成运行命令                 │
│  ├─ install_cmd()           # 生成安装命令                 │
│  ├─ log_cmd()               # 生成日志命令                 │
│  └─ devices()               # 设备列表 (Xcode)             │
│                                                            │
│  ┌──────────────────┐   ┌──────────────────┐              │
│  │  XcodeAdapter    │   │   NodeAdapter    │              │
│  │  ├─ schemes      │   │  ├─ scripts      │              │
│  │  ├─ bundle_id    │   │  └─ pkg manager  │              │
│  │  ├─ platform     │   │                  │              │
│  │  └─ devices      │   │                  │              │
│  └──────────────────┘   └──────────────────┘              │
│                                                            │
│  ConfigManager                                             │
│  ├─ 全局配置 (~/.vimo/dev-runner/config.json)             │
│  └─ 项目列表 (projects.json)                              │
│                                                            │
└────────────────────────────────────────────────────────────┘

┌─ dev-runner-app (保留，当前未使用) ───────────────────────┐
│  ProcessManager / ProcessMonitor                          │
│  可用于未来非终端方案或 Headless 场景                      │
└────────────────────────────────────────────────────────────┘
```

### Core 层职责（纯数据，无 IO）

| 模块 | 职责 | 输出 |
|------|------|------|
| **adapter** | 项目检测、解析 | `Command` 结构体 |
| **output** | 输出格式化 | 格式化后的字符串 |
| **config** | 配置读写 | 配置结构体 |
| **devices** | 设备列表 | `Device` 列表 |

**关键：Core 只生成 Command，不执行**

### App 层职责（有 IO、有状态）

| 模块 | 职责 |
|------|------|
| **process** | 执行 Command、捕获输出、管理生命周期 |
| **monitor** | CPU/内存监控 |
| **state** | App 状态管理、UI 同步 |
| **ffi** | Swift FFI 导出 |

### 统一类型（在 Core 中定义）

**Command（命令抽象）**：
```rust
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: HashMap<String, String>,
}
```

**Device（设备抽象）**：
```rust
pub struct Device {
    pub id: String,
    pub name: String,
    pub device_type: DeviceType,     // Mac | Simulator | Physical
    pub os_version: Option<String>,
    pub state: DeviceState,          // Available | Unavailable
}
```

**OutputEvent（输出事件，App 层使用）**：
```rust
pub struct OutputEvent {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub content: String,
}
```

### ETerm 如何使用 Core

```swift
// ETerm 插件示例
let adapter = XcodeAdapter.detect(path: projectPath)
let command = adapter.buildCmd(target: "MyApp", options: buildOptions)

// 直接在终端 tab 执行，不需要 ProcessManager
terminalTab.spawn(command.program, args: command.args)
```

---

## 持久化和日志

### 存储位置

```
~/.vimo/dev-runner/
├─ config.json              # 全局配置
├─ projects.json            # 项目列表
└─ logs/
   ├─ 2025-01-28/
   │  ├─ MyApp_build_103023.log
   │  ├─ MyApp_run_103045.log
   │  └─ node-server_run_110512.log
   └─ 2025-01-27/
      └─ ...
```

### config.json（全局配置）

```json
{
  "version": 1,
  "xcode": {
    "default_device": "iPhone 15 Pro",
    "default_config": "Debug",
    "output_format": "pretty"
  },
  "node": {
    "default_package_manager": "pnpm"
  },
  "log": {
    "retention_days": 7
  }
}
```

### projects.json（项目列表）

```json
{
  "version": 1,
  "projects": [
    {
      "path": "/Users/xxx/MyApp",
      "type": "xcode",
      "added_at": "2025-01-28T10:30:00Z",
      "last_used_at": "2025-01-28T14:20:00Z",
      "settings": {
        "last_scheme": "MyApp",
        "last_device": "iPhone 15 Pro",
        "last_config": "Debug"
      }
    },
    {
      "path": "/Users/xxx/node-server",
      "type": "node",
      "added_at": "2025-01-20T09:00:00Z",
      "last_used_at": "2025-01-28T11:00:00Z",
      "settings": {
        "last_script": "dev",
        "package_manager": "pnpm"
      }
    }
  ]
}
```

### 日志文件

**命名规则**：`{项目名}_{操作}_{HHMMSS}.log`

**内容格式**：
```
[2025-01-28T10:30:23.456Z] [INFO] Build started
[2025-01-28T10:30:24.123Z] [INFO] Compiling AppDelegate.swift
[2025-01-28T10:30:25.789Z] [ERROR] Cannot convert 'Int' to 'String'
```

**日志保留**：
- 按天分目录
- 默认保留 7 天
- 可在 config.json 中配置

### 配置共享

- 独立 App 和 ETerm 插件共享同一份配置（`~/.vimo/dev-runner/`）
- 读写时加文件锁，避免冲突

---

## Monorepo 处理

**原则：扫到什么展示什么**

不分析依赖关系，不管构建顺序。每个项目独立管理。

| 类型 | 扫到什么 | 能跑的条件 |
|------|----------|-----------|
| Xcode | `.xcodeproj` | 有 scheme |
| Node | `package.json` | 有 scripts |

**展示方式**：

```
┌─ 项目列表 ────────────────────┐
│ /monorepo                     │
│   ├─ apps/ios-app (xcode)     │
│   ├─ apps/web (node)          │
│   ├─ packages/shared (node)   │
│   └─ packages/sdk (xcode)     │
└───────────────────────────────┘
```

用户自己决定启动顺序。依赖关系复杂的场景，后续再考虑。

---

## 更新记录

- 2025-02-03: 文档与实现对齐
  - 明确 Swift App 使用终端直接执行方案（非 ProcessManager）
  - dev-runner-app 层保留但当前未使用
  - 实时监控 UI 暂未集成
  - ETerm 插件暂缓
- 2025-01-28: 明确两层 Rust 架构（core 共用 + app 专用）
- 2025-01-28: 初始设计文档，完成功能讨论
