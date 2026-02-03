# DevRunner 开发 TODO

> 按优先级排序

## 架构说明

```
dev-runner/
├── core/              # 共用层 ✅ 使用中
│   └── 项目检测、Command 生成、设备列表
├── app/               # App 专用层 ⏸️ 保留未使用
│   └── ProcessManager、ProcessMonitor
└── swift-app/         # Swift UI ✅ 已完成基础功能
    └── 内嵌终端直接执行命令
```

**实际方案**：Swift App 使用内嵌终端 (PTY) 执行命令，Rust 层只生成 Command。

---

## High Priority

### Core 层 ✅ 基本完成
- [x] 解析 xcodeproj 获取 schemes
- [x] 解析 bundle id
- [x] simctl list 获取模拟器列表
- [x] devicectl list 获取真机列表
- [x] build_cmd（支持 destination、derivedDataPath）
- [x] run_cmd（simctl launch / devicectl launch / open）
- [x] install_cmd（simctl install / devicectl install）
- [x] log_cmd（simctl spawn log / devicectl syslog / log stream）
- [ ] 输出格式化（xcbeautify 集成）— 优先级降低，终端方案下非必需

### App 层 ✅ 已实现（当前未使用）
- [x] 创建 app crate 结构
- [x] ProcessManager 实现
- [x] ProcessMonitor (CPU/内存监控)
- [x] FFI 导出
- [ ] ~~timeout 支持~~ — 暂不需要

## Medium Priority

### UI 完善
- [x] 运行状态指示（终端 tab 运行中/已停止）
- [x] 任务列表视图（TaskListView）
- [x] 新建任务弹窗（NewTaskSheet）
- [x] 任务驱动的布局（移除顶部按钮栏）
- [ ] 错误信息 toast 显示
- [ ] 最近使用的 scheme/device 记忆

### 配置打通
- [ ] `BuildOptions` 默认值从 Config 注入
- [ ] `RunOptions` 默认值从 Config 注入
- [ ] Swift 侧读写 projects.json

## Low Priority

### 代码质量
- [ ] 定义 `AdapterError` 类型
- [ ] `Command::to_string()` 转义处理
- [ ] `NodeAdapter::build_cmd` target 参数处理

### 后续考虑
- [ ] 实时监控 UI（需获取 PTY 子进程 PID）
- [ ] 日志文件落盘
- [ ] xcbeautify 输出格式化
- [ ] ETerm 插件集成

## 测试补充

- [ ] Xcode/Node adapter 异常路径
- [ ] 配置加载失败处理

---

## 当前进度

**Rust 层**：
- [x] Core: RunnerAdapter trait 定义
- [x] Core: XcodeAdapter - scheme/bundle_id/platform 解析
- [x] Core: XcodeAdapter - 设备列表（simctl/devicectl）
- [x] Core: XcodeAdapter - build/install/run/log 命令生成
- [x] Core: NodeAdapter (package.json/scripts/包管理器检测)
- [x] Core: ConfigManager
- [x] Core: OutputEvent
- [x] App: ProcessManager（启动、停止、输出捕获）
- [x] App: ProcessMonitor（CPU/内存监控）
- [x] App: FFI 导出
- [x] 测试覆盖（Core 33 + App 13 = 46 个通过）

**Swift 层**：
- [x] 独立 App 骨架
- [x] HSplitView 左右布局
- [x] SidebarView 树形项目列表
- [x] 内嵌终端 (MultiTerminalView)
- [x] FFI 桥接 (DevRunner.swift)
- [x] Build/Run/Stop 功能
- [x] Scheme/Device 选择器
- [x] Task-Tab 管理系统 (TaskKey/TaskAction)
- [x] TaskListView 任务列表（运行状态、资源监控）
- [x] NewTaskSheet 新建任务弹窗
- [x] 任务驱动的 UI 布局（VSplitView: 任务列表 + 终端）

**下一步**：
1. ~~ETerm 插件集成~~ — 暂缓
2. UI 细节打磨
3. 快捷键支持（Cmd+B 编译、Cmd+R 运行等）
