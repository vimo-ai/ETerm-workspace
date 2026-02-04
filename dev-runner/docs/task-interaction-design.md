# DevRunner 任务交互设计分析

## 一、现状问题

### 启动流程过重

当前启动一个任务需要 4 步：

```
选项目 → 点 New → 弹窗配置 Action/Target/Device → 点 Start
```

用户在侧边栏已经选了项目（系统已自动加载 target/device），弹窗本质上在重复确认已有信息。

### 任务列表无归属

所有任务平铺在 main 区域上半部分，不分项目。ETerm 的 build、DevRunner 的 run、随手开的 shell 全是同级。列表越长越难找。

### 状态模型过粗

只有 `isRunning: Bool`（绿点/灰点），无法区分：
- 成功完成 vs 失败退出
- 从未启动 vs 已结束
- 用户手动停止 vs 自然结束

### 动作与对象脱节

项目/target 在侧边栏只能"看"不能"操作"，没有直接的 run/build 入口。操作必须通过全局 New 按钮发起。

---

## 二、场景约束

### 项目规模

- 用户可能把所有项目都加进来，几十个项目
- 跨项目并发是常态（前后端联调、monorepo 多 package）

### 运行时多样性

| 运行时 | 项目标识 | 动作 |
|--------|---------|------|
| Swift/Xcode | .xcodeproj | build, run (需选 device) |
| Node | package.json | scripts (dev, build, test, lint...) |
| Rust | Cargo.toml | build, run, test |
| Docker | docker-compose.yml | up, down, build |
| Android | build.gradle | build, run (需选 device) |

### 任务生命周期分类

**短命型（有终态）**
```
cargo build, xcodebuild, pnpm test, cargo test, pnpm lint
```
用户关心：成功还是失败，耗时多少。

**长驻型（无终态）**
```
pnpm dev, cargo run (server), docker compose up, simulator app
```
用户关心：在不在跑，端口多少，日志输出。

**开放型（shell）**
```
zsh
```
用户自己敲命令，无预设目的。

### 并发模式

**单项目内**
- 改代码跑一下：1 task
- 边跑边测：2 tasks（dev + test --watch）
- 全流程：3+ tasks（dev server + test watcher + storybook）

**跨项目**
- 前后端联调：frontend(dev :3000) + backend(dev :8080)
- 全栈开发：frontend + backend + docker(db)
- FFI 依赖链：先 cargo build memex-rs → 再 build ETerm
- monorepo：daemon + server + shared-core 各自跑

**单命令多进程**
- docker compose up 起多个容器
- turbo run dev 跑多个 package
- 从终端角度：1 PTY = 1 进程树 = 1 task，不需要拆

### 任务数量

- 活跃任务数无上限
- MCP / Control API 可自动创建任务（AI agent 批量触发）
- 人手动 + AI 后台混合场景并发数不可预测
- 手动开发场景 6-8 个项目各自在跑已经很常见

---

## 三、设计约束

| # | 约束 | 来源 |
|---|------|------|
| C1 | 任务必须有项目归属 | 跨项目并发是常态 |
| C2 | 长驻型和短命型需要不同状态表达 | 生命周期不同 |
| C3 | 启动必须极低摩擦（1 步） | 最高频操作 |
| C4 | 单命令多进程不需要拆 | 1 PTY = 1 task |
| C5 | 串行依赖由人工判断 | 系统不编排 |
| C6 | 同配置可重跑，复用 task 槽位 | build-fix-rebuild 循环 |
| C7 | 活跃任务数无上限 | AI + MCP + 多项目并发 |
| C8 | 任务列表必须能缩放（分组/折叠/过滤） | 承接 C7 |
| C9 | 任务来源不只是手动，MCP 可自动创建 | Control API 场景 |

---

## 四、用户核心操作

按频率排序：

| 操作 | 频率 | 当前体验 |
|------|------|---------|
| 启动任务 | 最高频 | 4 步，太重 |
| 重跑任务 | 第二高频 | 无 restart，需重新走 New 流程 |
| 查看状态 | 持续 | 只有 running/not running |
| 聚焦输出 | 持续 | 需手动选 tab |
| 停止任务 | 中频 | Ctrl+C，无确认反馈 |
| 开 shell | 低频 | 需走 New 弹窗 |

---

## 五、技术现状（终端执行引擎）

### 确定不变的

- 终端（PTY）是执行引擎，命令通过 `sendCommand` 发送到 shell
- 多终端通过 `SimpleTerminalPoolWrapper` 管理
- Metal 渲染终端输出
- `TerminalTabManager` 管理 tab 生命周期

### 需要补的观测层

当前靠 `ps`/`lsof` 每 1-3 秒轮询猜测进程状态。缺失：
- 命令开始/结束时间
- 退出码（success/fail）
- 任务生命周期状态机

**建议方案：命令包装 + 状态机**

不换执行引擎，在发命令时包一层 OSC 标记获取退出码：
```
xcodebuild ...; printf '\033]777;task_exit;%d\007' $?
```

状态机：
```
idle → sent → running → completed(exitCode)
```

转换信号：
- idle → sent：sendCommand 那一刻
- sent → running：hasRunningProcess 变 true
- running → completed：hasRunningProcess 变 false + 解析 exitCode

---

## 六、方向共识

### 启动简化

- 项目/target 上直接放操作按钮（▶ / 🔨）
- 默认用上次的 target/device，点击直接跑（1 步）
- NewTaskSheet 降级为"高级配置"入口

### 任务组织

- 按项目分组，不是全局平铺
- 支持全局视图切换（看所有项目的任务）
- 大量任务时支持折叠/过滤（状态、类型）

### 状态表达

| 状态 | 视觉 | 含义 |
|------|------|------|
| idle | ⚫ 灰 | 槽位存在，未在运行 |
| sent | 🟡 黄 | 命令已发出，等待启动 |
| running | 🟢 绿 | 进程在跑 |
| success | ✅ 绿勾 | 退出码 0 |
| failed | ❌ 红叉 | 退出码非 0 |
| stopped | ⏹ 灰方 | 用户手动停止 |

完成的任务附带耗时信息，提供 restart 按钮。

### 扩展能力

- ⌘K Command Bar：快速搜索/启动任务
- 批量操作：停止全部、清理完成
- 失败置顶：大量任务时优先展示异常
- MCP 兼容：Control API 创建的任务和手动任务统一管理

---

## 七、参考产品

| 产品 | 借鉴点 |
|------|--------|
| Docker Desktop | 全局视图 + 项目分组 + 端口 badge + 一键 stop |
| PM2 | 简洁状态列 + restart/stop 常驻按钮 + exit code |
| Xcode | 默认 target/device，Run 一键，改配置走高级入口 |
| VS Code Tasks | Quick Task Picker + 最近使用任务列表 |
