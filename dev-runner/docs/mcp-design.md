# DevRunner MCP 设计文档

> MCP 作为 dev-runner 的遥控器，让 AI 管理开发项目

## 概述

DevRunner MCP Server 不自己执行命令，而是通过 API 控制 dev-runner（Swift App / ETerm 插件）。用户在 dev-runner UI 看到所有操作，不是黑盒。

```
Claude Code
    │ stdio (MCP 协议)
    ▼
dev-runner-mcp (遥控器)
    │ HTTP (localhost)
    ▼
dev-runner (执行者)
├── 终端执行命令
├── 进程状态
├── 日志 buffer
└── UI 显示给用户
```

## 设计原则

1. **MCP 是遥控器** - 不执行命令，只发指令
2. **dev-runner 是执行者** - 状态、日志、进程都在这里
3. **用户可见** - 所有操作在 UI 显示，不是黑盒
4. **简单优先** - HTTP 轮询，不搞推送
5. **幂等友好** - 重复操作返回当前状态，不报错

---

## 架构

### 整体架构

```
┌─────────────────────────────────────────────────────────────┐
│                      Claude Code                             │
└─────────────────────────────────────────────────────────────┘
                              │
                              │ stdio (JSON-RPC)
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    dev-runner-mcp                            │
│                                                              │
│  MCP Tools:                                                  │
│  - list_projects, add_project, remove_project               │
│  - start, stop, build, run                                  │
│  - status, logs                                             │
│  - list_devices, boot_simulator                             │
│                                                              │
│  只是 HTTP API 的 MCP 包装，超薄                             │
└─────────────────────────────────────────────────────────────┘
                              │
                              │ HTTP (localhost:9274)
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      dev-runner                              │
│                (Swift App / ETerm 插件)                      │
│                                                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │                 Control API Server                   │    │
│  │                 (HTTP, port 9274)                    │    │
│  └─────────────────────────────────────────────────────┘    │
│                              │                               │
│  ┌───────────────┬───────────┴───────────┬─────────────┐    │
│  │               │                       │             │    │
│  ▼               ▼                       ▼             ▼    │
│ Project      Process       Terminal    Log          UI      │
│ Registry     Manager       (PTY)       Buffer               │
│                                          │                  │
│                                    ┌─────┴─────┐            │
│                                    │ 写入时    │            │
│                                    │ strip ANSI│            │
│                                    │ 存纯文本  │            │
│                                    └───────────┘            │
└─────────────────────────────────────────────────────────────┘
```

### 组件职责

| 组件 | 职责 |
|------|------|
| **dev-runner-mcp** | MCP Server，把 MCP Tools 转成 HTTP 调用 |
| **Control API** | dev-runner 内的 HTTP Server，接收控制命令 |
| **Project Registry** | 管理项目列表 |
| **Process Manager** | 进程生命周期（启动、停止、状态） |
| **Terminal** | PTY 执行命令，渲染到 UI |
| **Log Buffer** | 捕获终端输出，存纯文本，提供查询 |

---

## 通信设计

### 协议选择

**HTTP (localhost:9274)**

- 简单，调试方便
- Swift 生态成熟
- 轮询日志，不搞 WebSocket/SSE

### dev-runner 没运行时

```
MCP 收到命令
    │
    ▼
检测 dev-runner 是否运行 (尝试连接 HTTP)
    │
    ├─ 运行中 → 发送命令
    │
    └─ 没运行 → open -a DevRunner
                    │
                    ▼
               等待最多 5 秒，重试连接
                    │
                    ├─ 成功 → 发送命令
                    │
                    └─ 失败 → 返回错误
                              "无法连接 dev-runner，请手动启动"
```

---

## API 设计

### 基础信息

- **Base URL**: `http://localhost:9274/api/v1`
- **Content-Type**: `application/json`
- **路径规范化**: 服务端对所有 path 参数做 `realpath` 处理，防止重复注册

### 健康检查

```
GET /health

Response:
{
  "status": "ok",
  "version": "0.1.0",
  "uptime_secs": 3600
}
```

### 项目管理

#### 列出项目
```
GET /projects

Response:
{
  "projects": [
    {
      "path": "/Users/xxx/MyApp",
      "name": "MyApp",
      "type": "xcode",
      "status": "running",
      "pid": 12345,
      "uptime_secs": 3600
    }
  ]
}
```

#### 添加项目
```
POST /projects
{
  "path": "/Users/xxx/MyApp"
}

Response:
{
  "path": "/Users/xxx/MyApp",
  "name": "MyApp",
  "type": "xcode",
  "targets": ["MyApp", "MyAppTests"]
}
```

#### 移除项目
```
DELETE /projects?path=/Users/xxx/MyApp

Response:
{ "success": true }
```

### 生命周期控制

#### 启动项目 (build + install + run)
```
POST /projects/start
{
  "path": "/Users/xxx/MyApp",
  "target": "MyApp",           // 可选，默认第一个
  "device": "iPhone 16 Pro",   // 可选，Xcode 项目
  "config": "Debug",           // 可选，默认 Debug
  "clean": false               // 可选，是否 clean build
}

Response:
{
  "success": true,
  "pid": 12345,
  "message": "MyApp started on iPhone 16 Pro"
}

// 如果项目已在运行，返回当前状态（幂等）
Response (already running):
{
  "success": true,
  "pid": 12345,
  "message": "MyApp is already running",
  "already_running": true
}
```

#### 停止项目
```
POST /projects/stop
{
  "path": "/Users/xxx/MyApp",
  "force": false               // 可选，true = SIGKILL
}

Response:
{
  "success": true,
  "exit_code": 0
}
```

#### 只构建
```
POST /projects/build
{
  "path": "/Users/xxx/MyApp",
  "target": "MyApp",
  "config": "Debug",
  "clean": false
}

Response:
{
  "success": true,
  "duration_secs": 45
}
```

#### 只运行 (不构建)
```
POST /projects/run
{
  "path": "/Users/xxx/MyApp",
  "target": "MyApp",
  "device": "iPhone 16 Pro"
}

Response:
{
  "success": true,
  "pid": 12345
}
```

### 状态查询

#### 获取状态
```
GET /projects/status?path=/Users/xxx/MyApp

Response:
{
  "status": "running",         // running | stopped | crashed
  "pid": 12345,
  "uptime_secs": 3600,
  "exit_code": null,           // 停止时有值
  "crashed_at": null           // 崩溃时有值
}
```

#### 获取日志
```
GET /projects/logs?path=/Users/xxx/MyApp&limit=100&since=12345&search=error

参数:
- path: 项目路径
- limit: 返回行数，默认 100
- since: 从哪个 sequence 开始（用于增量获取）
- search: 搜索关键词

Response:
{
  "lines": [
    {"seq": 12346, "text": "2025-02-04 10:23:45 App launched"},
    {"seq": 12347, "text": "2025-02-04 10:23:46 Loading config..."}
  ],
  "next_seq": 12348,           // 下次请求用这个值作为 since
  "has_more": true,
  "truncated": false           // true 表示旧日志已被丢弃
}
```

### 设备管理 (Xcode)

#### 列出设备
```
GET /devices?type=simulator

Response:
{
  "devices": [
    {
      "id": "ABC-123",
      "name": "iPhone 16 Pro",
      "type": "simulator",
      "os_version": "18.0",
      "state": "booted"
    }
  ]
}
```

#### 启动模拟器
```
POST /devices/boot
{
  "id": "ABC-123"
}

Response:
{ "success": true }
```

---

## 日志设计

### 日志捕获流程

```
终端 PTY 输出
      │
      ▼
  渲染到 UI (原始 ANSI)
      │
      ▼
  Strip ANSI (实时)
      │
      ▼
  处理 \r 覆盖行 (进度条等)
      │
      ▼
  写入 Log Buffer (纯文本 + sequence)
      │
      ▼
  API 查询返回
```

### `\r` 覆盖行处理

终端进度条使用 `\r` (回车) 覆盖当前行：
```
Downloading... 10%\r
Downloading... 50%\r
Downloading... 100%
```

**处理策略**：遇到 `\r` 时替换当前行，而不是追加新行。这样日志不会出现进度条残影。

### Log Buffer 设计

```swift
class LogBuffer {
    struct LogLine {
        let seq: UInt64          // 单调递增的序列号
        let text: String
    }

    private var lines: [LogLine] = []
    private var nextSeq: UInt64 = 1
    private var currentLine: String = ""  // 处理 \r 覆盖
    private let maxLines = 10000

    func append(_ rawData: Data) {
        let text = String(data: rawData, encoding: .utf8) ?? ""
        let plain = stripANSI(text)

        for char in plain {
            if char == "\r" {
                // 回车：重置当前行（下次写入会覆盖）
                currentLine = ""
            } else if char == "\n" {
                // 换行：提交当前行
                commitLine(currentLine)
                currentLine = ""
            } else {
                currentLine.append(char)
            }
        }
    }

    private func commitLine(_ text: String) {
        let line = LogLine(seq: nextSeq, text: text)
        nextSeq += 1
        lines.append(line)

        if lines.count > maxLines {
            lines.removeFirst()
        }
    }

    func getLines(since: UInt64?, limit: Int, search: String?) -> LogResult {
        var result = lines
        if let since = since {
            result = result.filter { $0.seq > since }
        }
        if let search = search {
            result = result.filter { $0.text.contains(search) }
        }
        return LogResult(
            lines: Array(result.prefix(limit)),
            nextSeq: nextSeq,
            hasMore: result.count > limit,
            truncated: lines.first?.seq ?? 1 > 1  // 旧日志已丢弃
        )
    }
}
```

### 容量控制

| 配置 | 值 | 说明 |
|------|-----|------|
| maxLines | 10000 | 每项目最多 1 万行 |
| 超出处理 | 丢弃旧行 | Ring buffer |
| 持久化 | 可选 | 落盘到 `~/.vimo/dev-runner/logs/` |

---

## MCP Tools 设计

### Tools 列表

| Tool | 说明 | 对应 API |
|------|------|----------|
| `health` | 健康检查 | GET /health |
| `list_projects` | 列出项目 | GET /projects |
| `add_project` | 添加项目 | POST /projects |
| `remove_project` | 移除项目 | DELETE /projects |
| `start` | 启动项目 | POST /projects/start |
| `stop` | 停止项目 | POST /projects/stop |
| `build` | 只构建 | POST /projects/build |
| `run` | 只运行 | POST /projects/run |
| `status` | 获取状态 | GET /projects/status |
| `logs` | 获取日志 | GET /projects/logs |
| `list_devices` | 列出设备 | GET /devices |
| `boot_simulator` | 启动模拟器 | POST /devices/boot |

### 示例 Tool 定义

```json
{
  "name": "start",
  "description": "启动项目 (build + install + run)",
  "inputSchema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "项目路径"
      },
      "target": {
        "type": "string",
        "description": "目标名 (scheme/script)"
      },
      "device": {
        "type": "string",
        "description": "设备名 (Xcode 项目)"
      },
      "config": {
        "type": "string",
        "description": "构建配置",
        "default": "Debug"
      }
    },
    "required": ["path"]
  }
}
```

---

## dev-runner 改动

### 需要新增的组件

#### 1. Control API Server

在 Swift App 中新增 HTTP Server：

```swift
import Vapor  // 或其他轻量 HTTP 框架

class ControlAPIServer {
    let app: Application
    let port: Int = 9274

    func start() {
        // 注册路由
        app.get("api", "v1", "projects") { req in
            // 返回项目列表
        }

        app.post("api", "v1", "projects", "start") { req in
            // 启动项目
        }

        // ... 其他路由
    }
}
```

#### 2. Log Buffer (每项目)

```swift
class ProjectManager {
    var projects: [String: Project] = [:]  // path -> Project

    struct Project {
        let path: String
        let adapter: RunnerAdapter
        var status: ProcessStatus
        var logBuffer: LogBuffer
    }
}
```

#### 3. 终端输出捕获

在终端层添加回调：

```swift
// 现有：只渲染
terminal.onOutput = { data in
    self.terminalView.write(data)
}

// 改成：渲染 + 捕获
terminal.onOutput = { data in
    self.terminalView.write(data)
    self.currentProject?.logBuffer.append(data)  // 新增
}
```

### ETerm 插件模式

如果是 ETerm 插件，需要 ETerm 提供：

```swift
protocol TerminalOutputCapture {
    /// 启用输出捕获
    var captureEnabled: Bool { get set }

    /// 输出回调
    var onOutput: ((Data) -> Void)? { get set }
}
```

DevRunnerKit 插件使用：

```swift
class DevRunnerPlugin {
    func setupCapture(terminal: TerminalOutputCapture) {
        terminal.captureEnabled = true
        terminal.onOutput = { [weak self] data in
            self?.logBuffer.append(data)
        }
    }
}
```

---

## 目录结构

```
dev-runner/
├── core/                      # 现有，项目检测、命令生成
│
├── mcp/                       # 新增：MCP Server
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs            # MCP Server 入口
│       ├── tools.rs           # MCP Tools 定义
│       └── client.rs          # HTTP 客户端 (调用 dev-runner)
│
├── swift-app/                 # 现有：独立 App
│   └── Sources/
│       ├── App/
│       ├── API/               # 新增：Control API Server
│       │   ├── APIServer.swift
│       │   ├── Routes.swift
│       │   └── Handlers.swift
│       └── Log/               # 新增：日志捕获
│           └── LogBuffer.swift
│
└── docs/
    └── mcp-design.md          # 本文档
```

---

## 使用流程

### 安装

```bash
# 1. 编译 MCP Server
cd dev-runner/mcp
cargo build --release

# 2. 添加到 Claude Code
claude mcp add dev-runner -- /path/to/dev-runner-mcp
```

### 使用示例

```
用户: "帮我启动 ~/Projects/MyApp"

Claude:
1. list_devices() → 获取可用模拟器
2. add_project({path: "~/Projects/MyApp"}) → 添加项目
3. start({path: "~/Projects/MyApp", device: "iPhone 16 Pro"})
   → dev-runner 在终端执行 build + install + run
   → 用户在 UI 看到构建过程

Claude: "MyApp 已在 iPhone 16 Pro 上启动"
```

```
用户: "看看有没有报错"

Claude:
1. logs({path: "~/Projects/MyApp", limit: 100, search: "error"})
   → 从 dev-runner 的 log buffer 获取日志

Claude: "发现 2 个错误：
  - NetworkError: Connection refused
  - ConfigError: Missing API key"
```

---

## 实现计划

### Phase 1: 基础功能

- [ ] dev-runner: Control API Server
- [ ] dev-runner: Log Buffer
- [ ] mcp: 基础 Tools (list_projects, start, stop, logs)

### Phase 2: 完整功能

- [ ] mcp: 所有 Tools
- [ ] dev-runner: 进程状态管理
- [ ] dev-runner: 设备管理 API

### Phase 3: ETerm 集成

- [ ] ETerm: TerminalOutputCapture 协议
- [ ] DevRunnerKit: 使用 capture 能力

---

## 风险与限制

| 风险 | 应对 |
|------|------|
| dev-runner 没运行 | MCP 自动启动 + 重试 |
| 日志太长 | Ring buffer 限制 1 万行，API 返回 `truncated` 标记 |
| HTTP 端口冲突 | 可配置端口，默认 9274 |
| ETerm 核心改动 | 设计为可选能力，不影响现有功能 |
| 路径重复注册 | 服务端 realpath 规范化 |
| 并发重复启动 | 幂等处理，返回当前状态 |
| 进度条日志残影 | 处理 `\r` 覆盖行 |

---

## 安全说明

**MVP 阶段不做认证**，理由：
- 本机开发工具，localhost 风险可接受
- 认证增加复杂度，后续按需添加

如需加强安全，可选方案：
- Unix Domain Socket 替代 HTTP
- 本地 token 文件 (`~/.vimo/dev-runner/token`)
