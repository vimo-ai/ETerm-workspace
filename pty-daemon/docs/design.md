# PTY Daemon: 终端会话持久化架构

> 创建时间: 2026-02-07
> 更新时间: 2026-02-09
> 状态: 🚧 Phase 1+3 实现中（fd passing + shm + ETerm 集成已验证）
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
  ETerm: 持 dup(master_fd) → Machine → crosswords → render
  ETerm: 同时写 POSIX 共享内存 ring buffer（~10-50ns memcpy）
  daemon: 休眠，只保持原 master_fd 引用防止 PTY 关闭
  → 性能 ≈ 当前性能，仅多一次 memcpy

ETerm detached / 崩溃 / 退出：
  daemon: 检测 owner socket 断开 → 重新注册 master_fd 到 kqueue
  daemon: 读 master_fd 输出 → 写入同一个共享内存 ring buffer
  → 单 writer 模型：attached 时 ETerm 写，detached 时 daemon 写

ETerm 回来（reattach）：
  daemon: 发送 AttachReady(shm_name) + sendmsg dup(master_fd)
  ETerm: open(shm_name) → dump() 读取历史 → replay 到 crosswords
  ETerm: 开始 poll dup(master_fd)，同时写 shm
  daemon: 回到休眠
```

**关键设计决策**：

- **同一时刻只有一方在跑核心逻辑**，不存在双写、不存在热路径 IPC
- daemon 在 attached 时完全不在数据路径上（ETerm 持 dup(master_fd)）
- socket 仅走控制面（attach/detach/list/kill/winsize_update），数据面不走
- socket 挂了不影响终端（ETerm 已持有 dup_fd）
- 历史数据通过 POSIX 共享内存（shm_open + mmap）传递，不走 socket
- terminal_id 映射确保 reattach 时 tab 和 session 精确对应

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

**使用 POSIX 共享内存 raw bytes ring buffer**，不使用 token buffer。原因：

- token buffer 需要轻量 parser 常驻，违背 Tier 3 "零 CPU" 的目标
- terminal reset + replay 足以恢复到可用状态
- 全屏 TUI 程序（vim 等）本身会在 reattach 后 redraw

#### 共享内存 ring buffer（SharedRingBuffer，已实现）

使用 `shm_open` + `mmap` 实现跨进程共享，单 writer / 多 reader lock-free 设计：

```
内存布局（Apple Silicon 128-byte cache line 对齐）：
  Offset 0-127:    Header  { magic: u32("PTYD"), version: u32, capacity: u64, reserved }
  Offset 128-255:  Slot    { write_pos: AtomicU64, padding }   ← 独占 cache line
  Offset 256-383:  Slot    { total_written: AtomicU64, padding } ← 独占 cache line
  Offset 384+:     data[0..capacity]
```

- **容量**：默认 1MB（`DEFAULT_RING_SIZE`），足够存储几千行终端输出
- **写入**：单 writer 通过 Acquire/Release ordering 更新 write_pos + total_written
- **读取**：reader 通过 Acquire ordering 读取一致性快照，`dump()` 返回有序数据
- **命名**：`/ptyd-{session_uuid前8位hex}`，随 session 创建/销毁
- **持久性**：shm 对象在内核中存活，不受单个进程崩溃影响
- **历史传递**：reattach 时客户端直接 `SharedRingBuffer::open(shm_name).dump()`，不走 socket

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

### Attach 协议（当前实现）

简化的 attach 协议，利用 `dup(master_fd)` + 共享内存避免复杂握手：

```
ETerm                          daemon
  │                              │
  │──── Attach{session_id} ────→│
  │                              │ dup(master_fd)
  │                              │ 从 kqueue 注销 master_fd（休眠）
  │                              │ 记录 owner_fd = client_fd
  │←──── AttachReady ────────────│ { session_id, cols, rows, child_pid, shm_name }
  │←──── FD(SCM_RIGHTS) ────────│ sendmsg 传 dup(master_fd)
  │                              │
  │ SharedRingBuffer::open(shm) │
  │ dump() → replay 到 crosswords│
  │ 用 child_pid 构造 Pty        │
  │ 开始 poll dup(master_fd)     │
  │ pty_read 时写 shm            │ daemon 休眠（Tier 1）
```

**不丢字节保证**（通过 dup 实现）：

1. daemon 调用 `dup(master_fd)` 得到副本，通过 SCM_RIGHTS 发给 ETerm
2. daemon 从 kqueue 注销 master_fd（停止读），ETerm 开始读 dup_fd
3. 内核 PTY buffer 中的未读数据由 ETerm 从 dup_fd 读走（两个 fd 指向同一 PTY）
4. 不需要 QUIESCE/FD_READY 握手 — dup 保证同一时刻只有一方在读

**失败回滚**：如果 send_fd 失败，daemon 关闭 dup_fd，重新注册 master_fd 到 kqueue，恢复 Active 状态。

### Attach 协议（目标版本，待实现）

完整协议增加 grid snapshot 传输，用于精确恢复屏幕状态（依赖 Phase 2 terminal-core 抽取）：

```
ETerm                          daemon
  │                              │
  │──── Attach{session_id} ────→│
  │                              │ QUIESCE: 停止 Machine 解析
  │                              │ 继续读入临时 ring（防内核 buffer 溢出）
  │←──── AttachReady ────────────│ { session_id, shm_name, snapshot_len }
  │←──── GRID_SNAPSHOT ──────────│ 序列化 grid state（screen + cursor + modes）
  │←──── FD(SCM_RIGHTS) ────────│ sendmsg 传 dup(master_fd)
  │                              │
  │ 应用 snapshot + shm replay  │
  │ 开始 poll dup(master_fd)     │
  │──── FD_READY ─────────────→│ daemon 进入休眠（Tier 1）
```

### Detach 协议（已实现）

**主动 detach**：

```
ETerm                          daemon
  │                              │
  │──── Detach{id, cols, rows} →│
  │                              │ 重新注册 master_fd 到 kqueue
  │                              │ ioctl TIOCSWINSZ(master_fd)
  │←──── Detached{id} ──────────│
  │ 关闭 dup_fd                  │ 进入 Tier 2（读 master_fd → 写 shm）
  │ 停止 Machine                 │
```

**崩溃 detach**（已实现）：

- daemon 检测到控制 socket 断开（kqueue `EV_EOF`）
- 遍历 sessions，找到 `owner_fd == disconnected_fd` 的 session
- 清理残留的 `pending_dup_fd`（如果 attach 和 send_fd 之间断开）
- **立即** 重新注册 master_fd 到 kqueue（不能有延迟，否则内核 buffer 可能溢出）
- 进入 Active 状态，winsize 使用最后已知值
- daemon 开始读 master_fd → 写 shm

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

### Phase 1：独立 daemon 验证 ✅

- [x] 新建 `pty-daemon` crate（workspace 内，依赖 teletypewriter）
- [x] PTY 创建（`pty.rs`）/ Unix socket 控制面（`server.rs`）/ kqueue 事件循环
- [x] fd passing — `dup(master_fd)` via SCM_RIGHTS（`fd_passing.rs`）
- [x] 进程内 ring buffer（`ring_buffer.rs`）
- [x] POSIX 共享内存 ring buffer（`shared_ring.rs`）— 替代进程内 ring buffer 用于跨进程
- [x] 控制面协议：Create / Attach / Detach / List / Kill / WinsizeUpdate / Ping
- [x] CLI 工具：create / attach / detach / list / kill / ping
- [x] 崩溃 detach 自动接管（kqueue EV_EOF 检测 + 立即 rearm master_fd）
- [x] EVFILT_PROC 监听子进程退出 + waitpid 回收 zombie
- [x] Tart VM E2E 测试（16 tests，deploy-tart.sh）
- **验证结果**：fd passing 可行、dup 保证不丢字节、崩溃 detach 正常

### Phase 2：terminal-core 抽取

- [ ] 评估 rio-backend ↔ sugarloaf 耦合深度
- [ ] 从 rio-backend 拆出 crosswords + ansi parser + Machine → terminal-core crate
- [ ] 编译验证：ETerm 和 daemon 可以同时依赖 terminal-core
- [ ] daemon 接入 terminal-core，支持 Tier 2 模式（Machine + crosswords）
- [ ] Grid snapshot 序列化/反序列化

### Phase 3：ETerm 集成 🚧（部分完成）

- [x] 用 dup(master_fd) + child_pid 构造 `Pty`，喂给 Machine
- [x] TerminalPool 新增 daemon 模式创建终端的路径（`try_create_daemon_pty`）
- [x] Attach 协议接入 — 从 shm dump 历史 → replay 到 crosswords
- [x] Detach / 崩溃 detach 正常工作
- [x] WinsizeUpdate 控制消息
- [x] terminal_id 精确映射 reattach（tab ↔ session 一一对应）
- [x] ETerm 端写 shm（`pty_read` 中 `shared_ring.write()`）
- [x] Tart VM UI 验证通过（屏幕历史恢复、多 tab 顺序正确）
- [ ] 主动 detach UI（用户手动 detach tab）
- [ ] fallback 模式优化（daemon 不可用时降级策略）

### Phase 4：生命周期与 UX

- [ ] launchd plist 集成
- [ ] 崩溃恢复提示 UI
- [ ] session 超时清理
- [ ] 磁盘 snapshot（内容回看）
- [ ] 配置项（退出行为、超时时间等）

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
- [x] ~~`Pty` 结构体从 raw fd + child_pid 重建~~ — 已验证：`teletypewriter::create_pty_from_file(File::from_raw_fd(pty_fd), shell_pid)` 可行
- [x] ~~ring buffer 大小选择~~ — 当前 1MB（DEFAULT_RING_SIZE），UI 验证通过，可后续调整
- [ ] grid snapshot 跨 winsize 恢复 — cols/rows 变化时 snapshot 怎么处理（resize reflow？截断？）
- [ ] macOS App Sandbox 对 SCM_RIGHTS 的限制 — 当前非 App Store 分发不受影响，App Store 分发时需要 App Group container path 或退化为 relay 模式
- [x] ~~teletypewriter crate 的依赖引入方式~~ — pty-daemon 在 workspace 内，直接 path 依赖
- [ ] terminal_id 跨 ETerm 重启的稳定性 — 当前 terminal_id 是 ETerm 运行时分配的，重启后可能变化导致映射失败（已观察到 3 sessions / 2 tabs 的情况）
- [x] ~~共享内存 ring data 传输方式~~ — 不走 socket，客户端直接从 shm dump（避免 EAGAIN）
