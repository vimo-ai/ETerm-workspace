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

**展示**：
- 内联显示在运行中的项目旁边
- 不单独弹窗

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

## ETerm 集成

**入口**：
- MenuBar 放入口
- 点击打开 DevRunner Tab

**UI 形态**：
- 作为 ETerm 的 TabView
- 和终端 tab 并列

**控制面板**：
- DevRunner Tab 只做控制面板（项目选择、scheme/device 选择、按钮）
- 不内嵌日志显示

**终端增强**：
- 点击 Run → 新开终端 tab 执行构建命令
- 点击 Logs → 新开终端 tab 执行日志命令
- 利用 ETerm 已有的终端渲染、搜索、选择能力

```
┌─ ETerm ────────────────────────────────────────────────────┐
│ [🔨 DevRunner] [Build: MyApp] [Logs: MyApp]  ← 多个 Tab    │
├────────────────────────────────────────────────────────────┤
│                                                            │
│  DevRunner Tab: 控制面板                                    │
│  Build Tab: xcodebuild 输出（终端）                         │
│  Logs Tab: log stream 输出（终端）                          │
│                                                            │
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
│  ┌─────────────────────┐     ┌─────────────────────────────┐   │
│  │    独立 App         │     │      ETerm 插件              │   │
│  │  (DevRunner.app)    │     │    (DevRunnerKit)           │   │
│  └──────────┬──────────┘     └──────────────┬──────────────┘   │
│             │ FFI                           │ FFI              │
└─────────────┼───────────────────────────────┼──────────────────┘
              │                               │
┌─────────────┼───────────────────────────────┼──────────────────┐
│             ▼                               │                   │
│  ┌─────────────────────┐                    │                   │
│  │   dev-runner-app    │ ← App 专用中间层    │                   │
│  │  (进程管理、状态)    │                    │                   │
│  └──────────┬──────────┘                    │                   │
│             │                               │                   │
│             ▼                               ▼                   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                  dev-runner-core                         │   │
│  │           (项目检测、Command 生成、输出解析)              │   │
│  │                   App + ETerm 共用                       │   │
│  └─────────────────────────────────────────────────────────┘   │
│                          Rust 层                                │
└─────────────────────────────────────────────────────────────────┘
```

### 职责划分

| 层 | crate | 职责 | 特点 |
|----|-------|------|-----|
| **core** | `dev-runner-core` | 项目检测、Command 生成、输出格式化、设备列表、配置管理 | 纯数据，无 IO 副作用 |
| **app** | `dev-runner-app` | 进程生命周期、输出捕获、状态管理 | 有 IO、有状态 |

### ETerm 插件如何使用

ETerm 插件**只依赖 core**：
- 用 core 检测项目、生成 Command
- 用 ETerm 的 PTY 执行命令（不需要 app 层的 ProcessManager）
- 输出直接走终端，无需捕获

### 目录结构

```
ETerm/dev-runner/
├── core/                           # 底层 Rust (App + ETerm 共用)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── adapter/                # 项目适配器
│       │   ├── mod.rs
│       │   ├── traits.rs           # RunnerAdapter trait
│       │   ├── xcode/              # Xcode 适配器
│       │   │   ├── mod.rs          # 项目解析、scheme、bundle id
│       │   │   └── devices.rs      # 模拟器/真机列表
│       │   └── node/               # Node 适配器
│       │       └── mod.rs          # package.json 解析
│       ├── output/                 # 输出解析
│       │   └── mod.rs              # xcbeautify 格式化等
│       └── config/                 # 配置管理
│           └── mod.rs
│
├── app/                            # App 专用中间层 Rust
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── process/                # 进程管理
│       │   ├── mod.rs
│       │   ├── manager.rs          # 进程生命周期
│       │   └── monitor.rs          # CPU/内存监控
│       ├── state/                  # App 状态管理
│       │   └── mod.rs
│       └── ffi/                    # Swift FFI
│           └── mod.rs
│
├── swift-app/                      # 独立 macOS App (Swift)
│   ├── DevRunner.xcodeproj
│   └── DevRunner/
│       ├── App/
│       ├── Bridge/
│       └── Views/
│
└── DESIGN.md

ETerm/ETerm/Plugins/DevRunnerKit/   # ETerm 插件 (只依赖 core)
├── Package.swift
├── Libs/DevRunnerCore/             # core 的 FFI dylib
└── Sources/DevRunnerKit/
    ├── DevRunnerPlugin.swift
    └── Views/
```

---

## Runner 通用架构

### 分层设计

```
┌─ dev-runner-app (App 专用) ────────────────────────────────┐
│                                                            │
│  ProcessManager                 StateManager               │
│  ├─ start(command)              ├─ 项目状态                │
│  ├─ stop(process_id)            ├─ 运行状态                │
│  ├─ 输出捕获 → channel          └─ UI 状态同步             │
│  └─ 进程监控 (CPU/内存)                                    │
│                                                            │
└────────────────────────────────────────────────────────────┘
                              │ 依赖
                              ▼
┌─ dev-runner-core (共用) ───────────────────────────────────┐
│                                                            │
│  RunnerAdapter (trait)                                     │
│  ├─ detect(path)            # 检测项目类型                 │
│  ├─ targets()               # 可运行目标列表               │
│  ├─ build_cmd()             # 生成构建命令                 │
│  ├─ run_cmd()               # 生成运行命令                 │
│  ├─ log_cmd()               # 生成日志命令                 │
│  ├─ devices()               # 设备列表 (Xcode)             │
│  └─ format_output()         # 格式化输出                   │
│                                                            │
│  ┌──────────────────┐   ┌──────────────────┐              │
│  │  XcodeAdapter    │   │   NodeAdapter    │              │
│  │  ├─ schemes      │   │  ├─ scripts      │              │
│  │  ├─ bundle_id    │   │  └─ pkg manager  │              │
│  │  └─ devices      │   │                  │              │
│  └──────────────────┘   └──────────────────┘              │
│                                                            │
│  ConfigManager              OutputFormatter                │
│  ├─ 全局配置                ├─ xcbeautify                  │
│  └─ 项目配置                └─ 其他格式化                  │
│                                                            │
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

- 2025-01-28: 明确两层 Rust 架构（core 共用 + app 专用）
- 2025-01-28: 初始设计文档，完成功能讨论
