# DevRunner 开发 TODO

> 按优先级排序

## 架构说明

```
dev-runner/
├── core/              # 共用层 (App + ETerm)
│   └── 项目检测、Command 生成、设备列表
├── app/               # App 专用层
│   └── 进程管理、监控、FFI
└── swift-app/         # Swift UI (待创建)
```

---

## High Priority

### Core 层完善
- [x] 解析 xcodeproj 获取 schemes
- [x] 解析 bundle id
- [x] simctl list 获取模拟器列表
- [x] devicectl list 获取真机列表
- [x] build_cmd（支持 destination、derivedDataPath）
- [x] run_cmd（simctl launch / devicectl launch / open）
- [x] install_cmd（simctl install / devicectl install）
- [x] log_cmd（simctl spawn log / devicectl syslog / log stream）
- [ ] 输出格式化（xcbeautify 集成）

### App 层创建 (dev-runner-app)
- [x] 创建 app crate 结构
- [x] ProcessManager 实现
  - [x] 启动进程后异步读取 stdout/stderr
  - [x] 等待子进程退出 → 更新状态
  - [x] stop() 发送 SIGTERM/SIGKILL
- [x] ProcessMonitor (CPU/内存监控)
- [x] FFI 导出
- [ ] timeout 支持（可选）

## Medium Priority

### 错误处理统一
- [ ] 定义 `AdapterError` 类型
- [ ] `build_cmd/run_cmd/log_cmd` 返回 `Result<Command, AdapterError>`
- [ ] `targets()` 返回 `Result<Vec<RunTarget>, AdapterError>`
- [ ] `ConfigManager::load_*` 返回 Result 或记录错误

### 配置打通
- [ ] `BuildOptions` 默认值从 Config 注入
- [ ] `RunOptions` 默认值从 Config 注入

### 其他
- [ ] `env/args` 在 adapter 中实际使用
- [ ] scan 递归处理符号链接、深度控制
- [ ] 进程内存单位确认（sysinfo 返回的是 bytes 还是 KiB）

## Low Priority

- [ ] `Command::to_string()` 转义处理（或标注仅用于日志）
- [ ] `OutputEvent::new` 的 source 语义
- [ ] `NodeAdapter::build_cmd` target 参数处理

## 架构改进（后续考虑）

- [ ] 引入 `RunnerService` 编排层
  - adapter → command → process → output 串联
  - 生命周期管理
  - 输出解析
  - 日志落盘
  - 错误归一化

## 测试补充

- [ ] ProcessManager 启动/停止/输出/状态更新
- [ ] 配置加载失败/损坏 JSON 处理
- [ ] 递归扫描与排除目录
- [ ] Xcode/Node adapter 异常路径（无 scripts / 错误 JSON）
- [ ] 轻量集成测试：用 Command 启动 sleep/echo 验证

---

## 当前进度

**已完成**：
- [x] 两层架构设计（core 共用 + app 专用）
- [x] Core: RunnerAdapter trait 定义
- [x] Core: XcodeAdapter - scheme 解析
- [x] Core: XcodeAdapter - bundle id 解析
- [x] Core: XcodeAdapter - 设备列表（simctl/devicectl）
- [x] Core: XcodeAdapter - build/install/run/log 命令生成
- [x] Core: NodeAdapter 骨架
- [x] Core: ConfigManager 骨架
- [x] Core: OutputEvent 定义
- [x] App: ProcessManager（启动、停止、输出捕获）
- [x] App: ProcessMonitor（CPU/内存监控）
- [x] App: FFI 导出（vimo-ffi 模式，JSON 字符串传递）
- [x] 测试覆盖（Core 33 + App 13 = 46 个通过）
- [x] 端到端验证（ETerm.xcodeproj，12 个 e2e 测试通过）

**下一步**：
1. Swift App 骨架
2. ETerm 插件集成（使用 core 层）
