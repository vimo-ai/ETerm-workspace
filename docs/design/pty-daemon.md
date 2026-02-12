# PTY Daemon: 终端会话持久化架构

> 创建时间: 2026-02-07
> 状态: 📋 设计阶段
> 前置讨论: 2026-02-04 tmux/dtach 调研（session `80b45ae2`）
> 审阅: 2026-02-07 Codex 审阅 + 人工筛选

## 1. 目标

为 ETerm 添加终端会话持久化能力：

- **后台执行 + 按需显示**：任务沉入后台继续跑，完成或需要时拉起
- **崩溃恢复**：ETerm 崩溃不丢失正在运行的进程
- **跨重启保持**：显式 detach 的 session 在 ETerm 重启后可恢复

**核心约束**：

- 极致性能优先，attached 热路径零退步
- 尽量少的内存和 CPU 占用
- 默认行为不变（关 tab = 杀进程）

## 2. 核心架构：daemon 作为热备兜底层

daemon **不是中间 relay 层**，而是 **热备（hot-standby）兜底层**。

通过 fd passing（`sendmsg SCM_RIGHTS`）实现零开销切换：

```
ETerm attached（正常使用）：
  ETerm: 直持 PTY master fd → Machine → crosswords → render
  daemon: 休眠，只保持 fd 引用防止 PTY 关闭
  → 性能 = 当前性能，零退步

ETerm detached / 崩溃 / 退出：
  daemon: 接管 PTY fd → 按需跑 Machine+crosswords
  → 有内容感知能力，可推通知

ETerm 回来（reattach）：
  daemon: 序列化 grid snapshot → 传给 ETerm + sendmsg fd
  ETerm: 收 snapshot 恢复状态 → 直接接管 fd
  daemon: 回到休眠
```

**关键设计决策**：

- **同一时刻只有一方在跑核心逻辑**，不存在双写、不存在热路径 IPC
- daemon 在 attached 时完全不在数据路径上
- socket 仅走控制面（attach/detach/list/kill），数据面不走
- socket 挂了不影响终端（ETerm 已持有 fd）

## 3. 共享 terminal-core crate

从 `rio-backend` 中抽取纯终端逻辑，daemon 和 ETerm 共用同一份代码：

```
terminal-core (新 crate):
  ├── crosswords (terminal grid)
  ├── ansi parser
  ├── Machine<T: EventedPty, U: EventListener>
  └── event types
  ← 纯逻辑，零渲染依赖

rio-backend (现有，瘦身):
  ├── depends on terminal-core
  └── sugarloaf 渲染相关逻辑

pty-daemon (新 crate):
  ├── depends on terminal-core
  ├── depends on teletypewriter
  ├── socket 控制面协议
  └── session 管理 + ring buffer
  ← 不依赖 sugarloaf
```

两边用同一份 terminal-core，保证 grid state 序列化/反序列化完全对称。

**注意**：rio-backend 当前依赖 sugarloaf，拆分前需要评估耦合深度。这是整个项目工作量最大的部分。

## 4. 三层运行状态（Tier）

daemon 不对所有 session 一视同仁，按活跃程度分层管理：

| Tier | 条件 | daemon 行为 | CPU | 内存 |
|------|------|-------------|-----|------|
| **Tier 1** | ETerm attached | 休眠，只保持 fd 引用 | 零 | 极低（fd + 元数据） |
| **Tier 2** | 有活跃输出（刚 detach / 后台任务在跑） | 跑 Machine + crosswords | 按输出量 | grid + scrollback |
| **Tier 3** | 空闲（shell 等输入 / 长期闲置） | 只持 fd + ring buffer | 零（poll 阻塞） | fd + ring buffer |

### 状态转换

```
                   attach
  Tier 2/3 ─────────────────→ Tier 1
     ↑                           │
     │ detach/崩溃                │ detach/崩溃
     │                           ↓
  Tier 3 ←── 无输出超时 ─── Tier 2
```

- **Tier 1→2**：ETerm detach 或崩溃，daemon 检测到 socket 断开，**立即** rearm 读循环接管 fd，启动 Machine
- **Tier 2→3**：无输出超过 T_idle（如 60s），释放 crosswords 内存，切到 ring buffer 模式
- **Tier 3→2**：检测到新输出，重建 crosswords（从 ring buffer replay）
- **Tier 2/3→1**：ETerm 发起 attach 请求

### Tier 2→3 降级策略

降级时：

- 生成 grid snapshot 存入内存或磁盘（作为 checkpoint）
- 释放 crosswords grid 内存，停止 Machine 线程
- 切换到主线程 poll 模式（只检测 fd 可读性）

**防抖**：引入 cooldown timer（如 60s），降级后 60s 内不升级回 Tier 2，避免输出间歇性来时频繁切换。

### Tier 3→2 升级策略

升级时：

- 如果有 checkpoint snapshot：从 snapshot + ring buffer 中 checkpoint 之后的部分恢复（快）
- 如果无 checkpoint：ring buffer 前插 terminal reset 序列（`ESC[!p`），从干净状态开始 replay（简单可靠）

### Tier 3 ring buffer 设计

**使用 raw bytes ring buffer + terminal reset**，不使用 token buffer。原因：

- token buffer 需要轻量 parser 常驻，违背 Tier 3 "零 CPU" 的目标
- terminal reset + replay 足以恢复到可用状态
- 全屏 TUI 程序（vim 等）本身会在 reattach 后 redraw

## 5. Session 生命周期

### 默认行为不变

| 用户操作 | 行为 | 与当前差异 |
|----------|------|-----------|
| 关闭 tab | 杀进程，daemon 清理 session | **无变化** |
| 显式 detach | session 保留在 daemon | 新能力 |
| ETerm 崩溃 | session 保留，下次启动提示恢复 | 新能力 |
| ETerm 正常退出 | 可配置：默认杀掉 / 保留 | 默认无变化 |

### 自动销毁条件

| 条件 | 触发 | 行为 |
|------|------|------|
| shell 退出 | PTY EOF **+ kqueue EVFILT_PROC** | daemon 自动清理 |
| 空闲超时 | 可配置（如 24h）无输出且未 attach | 自动清理 |
| 崩溃恢复未认领 | 可配置（如 48h）未被 ETerm 恢复 | 自动清理 |
| 手动 kill | 用户命令 | 立即清理 |

**子进程监控**：不能只靠 PTY EOF 检测 shell 退出。daemon 使用 `kqueue EVFILT_PROC` 监听 child pid，主动 `waitpid` 回收，防止 zombie。

### 资源泄漏防护

daemon 定期自检：

- `pid` 已退出 → 清理
- `fd` 读到 EOF → 清理
- `mem_size` 超限 → 压缩 scrollback 或降级
- `fd count` 与 session count 一致性校验

## 6. 状态交接协议

### Attach 协议（daemon → ETerm）

```
ETerm                          daemon
  │                              │
  │──── ATTACH_BEGIN ──────────→│
  │                              │ 进入 QUIESCE：
  │                              │   停止 Machine 解析
  │                              │   继续短暂读入临时 ring（防内核 buffer 溢出）
  │←──── QUIESCE_ACK(seq) ──────│
  │                              │
  │←──── GRID_SNAPSHOT(seq) ────│ 序列化 grid state
  │                              │ （先传 screen+cursor+mode_flags，
  │                              │  scrollback 异步补发）
  │←──── SESSION_META ──────────│ child_pid, winsize, terminal modes
  │←──── FD(SCM_RIGHTS) ────────│ sendmsg 传 master_fd
  │                              │
  │ 应用 snapshot                │
  │ 用 child_pid 构造 Pty        │
  │ 开始 poll master_fd          │
  │──── FD_READY(seq) ─────────→│ 确认已开始读
  │                              │ 停止临时 ring 读取
  │                              │ 进入休眠（Tier 1）
  │──── ATTACH_COMMIT(seq) ────→│
```

**不丢字节保证**：

1. daemon QUIESCE 后 **继续读入临时 ring**（不解析），直到收到 `FD_READY`
2. ETerm 收到 fd 后开始 poll，发送 `FD_READY`
3. daemon 收到 `FD_READY` 后停止读，进入休眠
4. 临时 ring 中的数据在 QUIESCE 期间已经被 daemon 读走不会丢失
5. `FD_READY` 之后的新输出由 ETerm 直接从 fd 读取

**兜底超时**：如果 handoff 超过 2s 未收到 `FD_READY`，daemon 回退到 Tier 2 继续工作，通知 ETerm attach 失败重试。

### Detach 协议

**主动 detach**：

```
ETerm                          daemon
  │                              │
  │──── DETACH(winsize) ───────→│
  │                              │ 恢复读 master_fd
  │                              │ 设置 TIOCSWINSZ
  │←──── DETACH_ACK ────────────│
  │ 关闭 fd 副本                 │ 进入 Tier 2
  │ 停止 Machine                 │
```

**崩溃 detach**：

- daemon 检测到控制 socket 断开（`EVFILT_READ` EOF）
- **立即** rearm 读循环，恢复读 master_fd（不能有延迟，否则内核 buffer 可能溢出）
- 进入 Tier 2，winsize 使用最后已知值

### Winsize 同步

Attached 期间，ETerm 窗口 resize 时发送控制消息：

```
ETerm ──── WINSIZE_UPDATE(cols, rows) ────→ daemon
```

daemon 记录最新 winsize，用于崩溃恢复。daemon 不对 PTY 执行 `TIOCSWINSZ`（因为 attached 时 ETerm 直接操作 fd），只做记录。

### 携带信息

交接协议中必须包含：

- `child_pid` — ETerm 用来构造 `Pty` 结构体（macOS 无法从 master fd 获取 child pid）
- `winsize`（cols, rows）— daemon 只关心 cols/rows，width/height 是 UI 层面的，不参与 PTY
- `cursor_position` — 精确恢复光标
- `terminal_modes` — alternate screen、bracketed paste、mouse mode、kitty keyboard 等模式标记
- `parser_state` — 如果从 Tier 3 恢复
- `protocol_version` + `snapshot_version` — 版本不兼容时拒绝 attach，退化为 redraw-only

## 7. Grid Snapshot

### 序列化格式

版本化二进制格式：

```
Header:
  protocol_version: u16    // 协议版本
  snapshot_version: u16    // snapshot 格式版本
  cols: u16
  rows: u16
  scrollback_len: u32
  cursor_row: u16
  cursor_col: u16
  cursor_style: u8
  terminal_modes: u32      // bitfield: alternate_screen, bracketed_paste, mouse_mode, etc.
  palette_hash: u32
  timestamp: u64

Body:
  screen_cells:    zstd(RLE(cells))     // 当前屏幕
  screen_attrs:    zstd(RLE(attrs))     // 属性 + 颜色
  scrollback:      zstd(chunked_rows)   // 滚动区（可异步传输）
```

### 大小估算

| 组成部分 | 未压缩 | zstd 压缩后 |
|----------|--------|-------------|
| 120x40 screen | 40~80 KB | <10 KB |
| 10k 行 scrollback | 数 MB | 0.5~2 MB |

### 传输策略

分阶段传输，优化 attach 延迟：

1. **Phase 1**（必须，阻塞）：screen + cursor + modes + attributes → <10KB，<1ms
2. **Phase 2**（异步，非阻塞）：scrollback 分块补发 → 用户 attach 后立即可用，滚动区稍后补齐

### 版本兼容

- `protocol_version` 不兼容 → 拒绝 attach，提示升级 daemon/ETerm
- `snapshot_version` 不兼容 → 退化为 redraw-only attach（传 fd 但不传 snapshot，ETerm 从空白状态开始）

## 8. Daemon 进程管理

### 启动方式

**推荐：launchd on-demand（socket activation）**

```xml
<!-- ~/Library/LaunchAgents/com.vimo.eterm-daemon.plist -->
<dict>
    <key>Label</key>
    <string>com.vimo.eterm-daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>/path/to/pty-daemon</string>
    </array>
    <key>Sockets</key>
    <dict>
        <key>control</key>
        <dict>
            <key>SockPathName</key>
            <string>/tmp/eterm-daemon.sock</string>
        </dict>
    </dict>
    <key>KeepAlive</key>
    <dict>
        <key>OtherJobEnabled</key>
        <false/>
    </dict>
</dict>
```

- 有连接时自动拉起，无 session 时自动退出
- daemon 崩溃 launchd 自动重启
- 零常驻开销

### 崩溃处理

| 场景 | 影响 | 恢复策略 |
|------|------|----------|
| daemon 崩溃，ETerm attached | **无影响**（ETerm 持有 fd） | ETerm 检测到 daemon 不在，提示"后台不可用" |
| daemon 崩溃，有 detached session | **session 丢失**（daemon 的 fd 关闭→SIGHUP→shell 退出） | launchd 重启 daemon；磁盘 snapshot 仅供内容查看，**不能恢复交互**（fd 已丢失） |
| ETerm 崩溃 | daemon 接管所有 session | 下次 ETerm 启动提示恢复 |

**关键限制**：daemon 崩溃后，detached session 的 PTY fd 随进程死亡而关闭，macOS 没有跨进程恢复 fd 的机制。磁盘 snapshot 只能用于回看最后的屏幕内容，不能恢复活跃会话。因此 **daemon 稳定性是核心要求**，launchd KeepAlive 是必须的。

### 磁盘 snapshot（内容回看用）

daemon 定期（每 N 秒或每 M KB 输出）将 Tier 2 session 的 grid snapshot 写入磁盘：

```
~/.vimo/daemon/sessions/
  ├── {session_uuid}.snapshot   # 二进制 grid state
  └── {session_uuid}.meta       # pid, winsize, timestamp, owner_id
```

**用途**：

- daemon 崩溃后，ETerm 可以展示最后的屏幕快照（只读，不可交互）
- 调试 / 日志用途

## 9. 多实例与安全

### 单 daemon 共享

所有 ETerm 实例共享同一个 daemon 进程：

- 每个 session 有 `owner_id`（ETerm instance UUID）+ `last_attached_timestamp`
- 同一 session 同一时刻只允许一个 ETerm 实例 attach
- 已被 attach 的 session，其他实例请求时 daemon 返回 `ATTACH_DENY`
- owner 崩溃后（socket 断开），session 变为无主，其他实例可以接管

### 安全边界

- 控制面 Unix domain socket 路径权限 `0700`
- `getpeereid()` 校验对端 UID，拒绝非当前用户的连接
- 不同 macOS user 之间完全隔离（各自独立的 daemon 进程和 socket）

## 10. 与现有系统的关系

### 插件系统（VlaudeKit / MemexKit）

- **Attached（Tier 1）**：插件事件由 ETerm 通过 AICliKit 产生，行为不变
- **Detached（Tier 2/3）**：插件不工作。daemon 不承载插件运行时
- **Reattach 时**：插件恢复事件流

未来可选增强：daemon Tier 2 提供轻量通知钩子（非完整插件系统），例如检测 exit code != 0 推通知。

### 信号传递

- Attached 时：ETerm 直持 fd，Ctrl+C 等信号直接写入 PTY，daemon 不参与
- Detached 时：daemon 持有 fd，不处理信号输入（无前台交互）

### 现有 TerminalPool detach/attach

TerminalPool 已有的进程内 detach/attach（跨池迁移）继续保留，两者正交：

- **Pool detach/attach**：终端在 ETerm 窗口间移动（进程内）
- **Daemon detach/attach**：终端在 ETerm 进程和 daemon 之间移动（跨进程）

## 11. 线程模型

daemon 使用混合线程模型：

- **主线程**：kqueue 统一处理控制 socket + 所有 Tier 1/3 session 的 fd 监控
  - Tier 1：只监听 socket 断开（触发崩溃接管）
  - Tier 3：监听 fd 可读性（触发升级 Tier 2）
  - 使用 kqueue 而非 poll，O(1) 事件分发
- **Tier 2 worker 线程**：每个 Tier 2 session 独立线程跑 Machine
  - 与 ETerm 现有的 Machine 线程模型一致
  - session 降到 Tier 3 时线程退出

## 12. 实施路径

### Phase 1：独立 daemon 验证（worktree 隔离）

- 新建 `pty-daemon` crate（独立于 rio workspace，直接依赖 teletypewriter）
- 实现：PTY 创建 / Unix socket 控制面 / fd passing（含 child_pid 传递）/ ring buffer
- 控制面协议状态机 + attach/detach stress test
- 不依赖 terminal-core（先用纯 ring buffer 模式验证机制）
- CLI 工具：create / attach / detach / list / kill
- Tart VM 上验证
- **验证项**：fd passing 可行性、handoff 不丢字节、崩溃 detach 自动接管

### Phase 2：terminal-core 抽取

- 评估 rio-backend ↔ sugarloaf 耦合深度
- 从 rio-backend 拆出 crosswords + ansi parser + Machine → terminal-core crate
- 编译验证：ETerm 和 daemon 可以同时依赖 terminal-core
- daemon 接入 terminal-core，支持 Tier 2 模式（Machine + crosswords）
- Grid snapshot 序列化/反序列化

### Phase 3：ETerm 集成

- 用传过来的 fd + child_pid 构造 `Pty`，喂给 Machine
- TerminalPool 新增 daemon 模式创建终端的路径
- Attach/Detach 协议完整接入
- WINSIZE_UPDATE 控制消息
- 完整 attach/detach/崩溃恢复 E2E 测试

### Phase 4：生命周期与 UX

- launchd plist 集成
- 崩溃恢复提示 UI
- session 超时清理
- 磁盘 snapshot（内容回看）
- 配置项（退出行为、超时时间等）

---

## 附录 A：与 tmux/dtach 的对比

| 特性 | tmux | dtach | ETerm daemon |
|------|------|-------|-------------|
| 热路径性能 | 有 IPC 开销 | 有 IPC 开销 | **零退步**（fd passing） |
| 内容感知（detached） | ✅ | ❌ | ✅（Tier 2） |
| 完美 reattach | ✅ | ❌（需 redraw） | ✅（grid snapshot） |
| 资源占用（idle） | 较高 | 极低 | **极低**（Tier 3） |
| 双写 | server 单写 | relay 双拷贝 | **单写** |
| 崩溃隔离 | client 崩不影响 | client 崩不影响 | **双向隔离** |

## 附录 B：待确认问题

- [ ] rio-backend 对 sugarloaf 的依赖深度 — 决定 terminal-core 拆分的工作量（**Phase 2 最大风险**）
- [ ] `Pty` 结构体从 raw fd + child_pid 重建 — 确认 `Child` 结构体哪些字段必须、哪些可省
- [ ] ring buffer 大小选择 — 需要 benchmark 不同场景（编译输出、Claude 会话、cat 大文件）
- [ ] grid snapshot 跨 winsize 恢复 — cols/rows 变化时 snapshot 怎么处理（resize reflow？截断？）
- [ ] macOS App Sandbox 对 SCM_RIGHTS 的限制 — 当前非 App Store 分发不受影响，App Store 分发时需要 App Group container path 或退化为 relay 模式
- [ ] teletypewriter crate 的依赖引入方式 — Phase 1 的 pty-daemon 怎么依赖 rio workspace 下的 teletypewriter
