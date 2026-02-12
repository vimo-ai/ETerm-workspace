# DevRunner 生命周期与日志分离

> 日期: 2026-02-07
> 状态: **待讨论**
> 来源: session `e026fa97` 修复 LogBuffer 空行时暴露的问题

## 问题

当前 `simctl launch --console-pty` 把 app 生命周期绑定到了 PTY/terminal 进程：

```
DevRunner terminal → shell → simctl launch --console-pty → [PTY] → app
```

**后果**：DevRunner 重启 → terminal pool 销毁 → PTY master 关闭 → shell SIGHUP → simctl 被杀 → app 退出。

## 已修复（e026fa97）

| 文件 | 改动 |
|------|------|
| `rio/sugarloaf-ffi/src/infra/log_buffer.rs` | `\r\r\n` 处理修复 + partial UTF-8 缓冲 + 6 个测试 |
| `dev-runner/swift-app/Sources/API/ControlAPIServer.swift` | `handleStart`/`handleRun` 添加 `simctl terminate` |

## 改造方向

**原则**：app 生命周期由 DevRunner 管理，不依赖 PTY 连接。

### 方案：分离启动与日志

```
启动: simctl launch <device> <bundle-id>        # app 独立运行
停止: simctl terminate <device> <bundle-id>      # DevRunner 主动杀
日志: simctl spawn <device> log stream --predicate 'subsystem == "com.vimo.claude.Vlaude"'
```

### 需要解决

1. **日志捕获方式变更**
   - `--console-pty` 能捕获 `print()` stdout
   - `log stream` 只能捕获 `os_log` / `Logger`
   - Vlaude iOS 当前用 `print()` 打日志，需要迁移到 `os_log`

2. **DevRunner core 改造**
   - `run_cmd()` 去掉 `--console-pty`
   - 新增 `log_cmd()` 生成 `log stream` 命令（core 已有此方法）
   - `handleStart`/`handleRun` 启动后单独 attach log stream 管道

3. **DevRunner 重启恢复**
   - 重启后能发现已运行的 app（通过 `simctl listapps` 或 PID 检查）
   - 重新 attach log stream
   - 不需要重新 build/install

## 优先级

低。当前 `--console-pty` + LogBuffer 修复后 MCP logs 已能正常工作，只是 DevRunner 重启会杀 app。
