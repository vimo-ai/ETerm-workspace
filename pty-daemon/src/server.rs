//! Unix domain socket + WebSocket 控制面 server
//!
//! 单线程 kqueue 事件循环：
//! - 监听控制 socket 新连接（Unix domain socket）
//! - 监听 WebSocket 连接（TCP, for iOS/LAN remote attach）
//! - 处理客户端请求（Create/Attach/Detach/List/Kill/WinsizeUpdate）
//! - fd passing 直连模式：Unix socket Attach 时 dup(master_fd) 传给 ETerm，daemon 休眠
//! - WebSocket Attach：daemon 代理 PTY I/O，不休眠，支持多客户端同时 attach
//! - Detach/崩溃时 daemon 重新注册 master_fd，读输出写 ring_buffer
//! - 监听 attached session 的 owner socket 断开（崩溃检测）

use base64::Engine;
use crate::fd_passing;
use crate::protocol::{self, Push, PushAck, Request, Response, SessionInfo, PROTOCOL_VERSION};
use crate::pty;
use crate::session::{Session, SessionManager, SessionState, WinSize};
use crate::ws_server::{self, WsCommand, WsSharedState};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::RawFd;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Idle timeout: daemon exits after this duration with 0 sessions and 0 clients
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Baton-pass timeout: how long to wait for ETerm's DetachAck before force-proceeding
const BATON_PASS_TIMEOUT: Duration = Duration::from_secs(3);

/// kqueue 注册/注销（自由函数，避免 borrow 冲突）
fn kq_register(kq: RawFd, fd: RawFd, filter: i16, flags: u16) -> io::Result<()> {
    let ev = libc::kevent {
        ident: fd as usize,
        filter,
        flags,
        fflags: 0,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    let ret = unsafe { libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Daemon server
pub struct Server {
    socket_path: PathBuf,
    listener: UnixListener,
    sessions: SessionManager,
    /// client fd → read buffer
    clients: HashMap<RawFd, ClientState>,
    /// attach 时暂存 dup(master_fd)，send_attach_data 取走后传给 ETerm
    pending_dup_fds: HashMap<Uuid, RawFd>,
    running: bool,
    /// idle auto-exit: 0 sessions + 0 clients 时记录起始时刻
    idle_since: Option<Instant>,
    /// WebSocket command receiver (from ws_server thread)
    ws_cmd_rx: Option<std::sync::mpsc::Receiver<WsCommand>>,
    /// WebSocket shared state (subscriber management)
    ws_shared: Option<Arc<Mutex<WsSharedState>>>,
    /// Self-pipe for waking kqueue when WS commands arrive
    ws_wakeup_pipe: Option<(RawFd, RawFd)>, // (read_fd, write_fd)
}

struct ClientState {
    stream: UnixStream,
    read_buf: Vec<u8>,
}

impl Server {
    pub fn new(socket_path: &Path) -> io::Result<Self> {
        // 清理残留 socket
        if socket_path.exists() {
            std::fs::remove_file(socket_path)?;
        }

        // 确保父目录存在
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(socket_path)?;
        listener.set_nonblocking(true)?;

        // 设置 socket 权限 0700
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o700))?;
        }

        eprintln!("[daemon] listening on {}", socket_path.display());

        // Start WebSocket server on a background thread
        let (ws_cmd_rx, ws_shared) = ws_server::start_ws_server();

        // Create self-pipe for waking kqueue when WS commands arrive
        let mut pipe_fds = [0i32; 2];
        let pipe_ret = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
        let ws_wakeup_pipe = if pipe_ret == 0 {
            unsafe {
                // Set both ends non-blocking
                let flags = libc::fcntl(pipe_fds[0], libc::F_GETFL, 0);
                libc::fcntl(pipe_fds[0], libc::F_SETFL, flags | libc::O_NONBLOCK);
                let flags = libc::fcntl(pipe_fds[1], libc::F_GETFL, 0);
                libc::fcntl(pipe_fds[1], libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            Some((pipe_fds[0], pipe_fds[1]))
        } else {
            eprintln!("[daemon] warning: failed to create self-pipe for WS wakeup");
            None
        };

        Ok(Self {
            socket_path: socket_path.to_owned(),
            listener,
            sessions: SessionManager::new(),
            clients: HashMap::new(),
            pending_dup_fds: HashMap::new(),
            running: true,
            idle_since: Some(Instant::now()),
            ws_cmd_rx: Some(ws_cmd_rx),
            ws_shared: Some(ws_shared),
            ws_wakeup_pipe,
        })
    }

    /// 主事件循环（kqueue）
    pub fn run(&mut self) -> io::Result<()> {
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return Err(io::Error::last_os_error());
        }

        // 注册 listener fd
        kq_register(
            kq,
            self.listener.as_raw_fd(),
            libc::EVFILT_READ,
            libc::EV_ADD,
        )?;

        // 注册 WebSocket self-pipe 读端到 kqueue
        if let Some((read_fd, _)) = self.ws_wakeup_pipe {
            kq_register(kq, read_fd, libc::EVFILT_READ, libc::EV_ADD)?;
        }

        // 注册 SIGTERM/SIGINT — 用 kqueue EVFILT_SIGNAL 代替 signal handler
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }
        let sig_ev = |sig: libc::c_int| libc::kevent {
            ident: sig as usize,
            filter: libc::EVFILT_SIGNAL,
            flags: libc::EV_ADD,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let sig_events = [sig_ev(libc::SIGTERM), sig_ev(libc::SIGINT)];
        let ret = unsafe {
            libc::kevent(
                kq,
                sig_events.as_ptr(),
                sig_events.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut events: Vec<libc::kevent> = vec![unsafe { std::mem::zeroed() }; 64];

        eprintln!("[daemon] event loop started");

        while self.running {
            // 定期 reap 死掉的子进程（兜底，主要靠 EVFILT_PROC）
            let dead = self.sessions.reap_dead();
            for (id, master_fd) in &dead {
                let _ = kq_register(kq, *master_fd, libc::EVFILT_READ, libc::EV_DELETE);
                // Notify WS subscribers that the session ended
                if let Some(ref ws_shared) = self.ws_shared {
                    let mut state = ws_shared.lock().unwrap();
                    state.notify_session_ended(id);
                }
                eprintln!("[daemon] reaped dead session {id}");
            }

            // Drain WebSocket commands (non-blocking)
            self.drain_ws_commands(kq);

            // Idle auto-exit：0 sessions + 0 clients 持续 IDLE_TIMEOUT 后退出
            let ws_has_clients = self
                .ws_shared
                .as_ref()
                .map_or(false, |s| !s.lock().unwrap().client_sessions.is_empty());
            if self.sessions.count() == 0 && self.clients.is_empty() && !ws_has_clients {
                if let Some(since) = self.idle_since {
                    if since.elapsed() >= IDLE_TIMEOUT {
                        eprintln!("[daemon] idle timeout ({IDLE_TIMEOUT:?}), exiting");
                        self.running = false;
                        continue;
                    }
                } else {
                    self.idle_since = Some(Instant::now());
                }
            } else {
                self.idle_since = None;
            }

            // kqueue wait（1s 超时，用于定期维护）
            let timeout = libc::timespec {
                tv_sec: 1,
                tv_nsec: 0,
            };

            let nev = unsafe {
                libc::kevent(
                    kq,
                    std::ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    events.len() as i32,
                    &timeout,
                )
            };

            if nev < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }

            for i in 0..nev as usize {
                let ev = &events[i];
                let fd = ev.ident as RawFd;

                // Check if this is the WS wakeup pipe
                let is_ws_pipe = self
                    .ws_wakeup_pipe
                    .map_or(false, |(read_fd, _)| fd == read_fd);

                if ev.filter == libc::EVFILT_SIGNAL {
                    let sig = ev.ident as i32;
                    eprintln!("[daemon] received signal {sig}, shutting down");
                    self.running = false;
                    break;
                } else if is_ws_pipe {
                    // Drain the pipe and process WS commands
                    let mut drain_buf = [0u8; 256];
                    unsafe {
                        libc::read(fd, drain_buf.as_mut_ptr() as *mut _, drain_buf.len());
                    }
                    // Commands are drained at the top of the loop
                } else if fd == self.listener.as_raw_fd() {
                    self.accept_clients(kq)?;
                } else if self.clients.contains_key(&fd) {
                    let ev_filter = ev.filter;
                    let ev_flags = ev.flags;
                    let ev_data = ev.data;
                    if ev_flags & libc::EV_EOF != 0 {
                        eprintln!("[daemon] kqueue: fd={fd} EV_EOF (filter={ev_filter}, flags=0x{ev_flags:x}, data={ev_data})");
                        self.handle_client_disconnect(kq, fd);
                    } else {
                        // eprintln!("[daemon] kqueue: fd={fd} data ready (filter={ev_filter}, flags=0x{ev_flags:x}, data={ev_data})");
                        self.handle_client_data(kq, fd)?;
                    }
                } else if ev.filter == libc::EVFILT_PROC {
                    // child 进程退出事件
                    self.handle_child_exit(kq, ev.ident as libc::pid_t);
                } else {
                    self.handle_session_output(kq, fd);
                }
            }
        }

        unsafe {
            libc::close(kq);
        }
        self.cleanup();
        Ok(())
    }

    fn accept_clients(&mut self, kq: RawFd) -> io::Result<()> {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true)?;
                    let fd = stream.as_raw_fd();
                    // Enlarge send buffer for large AttachReady messages (snapshot data)
                    unsafe {
                        let buf_size: libc::c_int = 512 * 1024; // 512KB
                        libc::setsockopt(
                            fd,
                            libc::SOL_SOCKET,
                            libc::SO_SNDBUF,
                            &buf_size as *const _ as *const libc::c_void,
                            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                        );
                    }
                    eprintln!("[daemon] client connected fd={fd}");
                    kq_register(kq, fd, libc::EVFILT_READ, libc::EV_ADD)?;
                    self.clients.insert(
                        fd,
                        ClientState {
                            stream,
                            read_buf: Vec::new(),
                        },
                    );
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn handle_client_data(&mut self, kq: RawFd, fd: RawFd) -> io::Result<()> {
        let mut tmp = [0u8; 4096];

        // 先读数据
        let client = match self.clients.get_mut(&fd) {
            Some(c) => c,
            None => return Ok(()),
        };
        match client.stream.read(&mut tmp) {
            Ok(0) => {
                eprintln!("[daemon] fd={fd} read returned 0 (EOF from read path)");
                let _ = client;
                self.handle_client_disconnect(kq, fd);
                return Ok(());
            }
            Ok(n) => {
                client.read_buf.extend_from_slice(&tmp[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        }

        // 尝试解析完整消息
        loop {
            let buf_snapshot = self.clients[&fd].read_buf.clone();
            match protocol::try_decode_message::<Request>(&buf_snapshot) {
                Some((result, consumed)) => {
                    let response = match result {
                        Ok(req) => self.handle_request(kq, fd, req),
                        Err(e) => Response::Error {
                            message: format!("parse error: {e}"),
                        },
                    };

                    // 发送响应（大消息如 AttachReady+snapshot 需要 blocking write）
                    let resp_bytes = protocol::encode_message(&response);
                    if let Some(client) = self.clients.get_mut(&fd) {
                        client.stream.set_nonblocking(false).ok();
                        let _ = client.stream.write_all(&resp_bytes);
                        client.stream.set_nonblocking(true).ok();
                        client.read_buf.drain(..consumed);
                    }

                    // Attach 成功后发送 dup(master_fd)
                    if let Response::AttachReady { session_id, .. } = &response {
                        if let Err(e) = self.send_attach_data(fd, *session_id) {
                            eprintln!("[daemon] attach data send failed: {e}, reverting");
                            self.revert_attach(kq, *session_id);
                        }
                    }
                }
                None => break,
            }
        }

        Ok(())
    }

    fn handle_client_disconnect(&mut self, kq: RawFd, fd: RawFd) {
        eprintln!("[daemon] client disconnected fd={fd}");
        let _ = kq_register(kq, fd, libc::EVFILT_READ, libc::EV_DELETE);
        self.clients.remove(&fd);

        // Clear previous_owner_fd for any sessions that referenced this client.
        // This prevents baton-release attempts to a dead socket.
        for session in self.sessions.list_mut() {
            if session.previous_owner_fd == Some(fd) {
                session.previous_owner_fd = None;
            }
        }

        // 收集需要 crash detach 的 session id + master_fd
        let to_detach: Vec<(Uuid, RawFd)> = self
            .sessions
            .list()
            .iter()
            .filter(|s| s.owner_fd == Some(fd))
            .map(|s| (s.id, s.master_fd))
            .collect();

        for (id, master_fd) in to_detach {
            eprintln!("[daemon] crash detach session {id} (owner fd={fd} gone)");
            // 清理可能残留的 pending dup_fd（Attach 和 send_fd 之间断开）
            if let Some(dup_fd) = self.pending_dup_fds.remove(&id) {
                eprintln!("[daemon] closing leaked pending dup_fd={dup_fd} for session {id}");
                unsafe {
                    libc::close(dup_fd);
                }
            }
            if let Some(s) = self.sessions.get_mut(&id) {
                // Replay ring buffer history into terminal parser before transitioning.
                // ETerm was writing to shared_ring during Attached state — this data
                // represents the terminal content the user saw before the crash.
                let ring_data = s.shared_ring.dump();
                if let Some(ref mut ts) = s.terminal_state {
                    ts.replay_history(&ring_data);
                }
                s.detach(None);
            }
            // Re-register master_fd — daemon resumes reading shell output → ring buffer
            let _ = kq_register(kq, master_fd, libc::EVFILT_READ, libc::EV_ADD);
        }
    }

    fn handle_request(&mut self, kq: RawFd, client_fd: RawFd, req: Request) -> Response {
        match req {
            Request::Create {
                shell,
                cols,
                rows,
                working_dir,
                terminal_id,
                envs,
            } => self.handle_create(
                kq,
                shell,
                cols,
                rows,
                working_dir,
                terminal_id,
                envs.as_ref(),
            ),

            Request::Attach { session_id } => self.handle_attach(kq, client_fd, session_id),

            Request::Detach {
                session_id,
                cols,
                rows,
                grid_snapshot,
            } => self.handle_detach(kq, session_id, cols, rows, grid_snapshot),

            Request::List => self.handle_list(),

            Request::Kill { session_id } => self.handle_kill(kq, session_id),

            Request::WinsizeUpdate {
                session_id,
                cols,
                rows,
            } => self.handle_winsize_update(session_id, cols, rows),

            Request::Ping => Response::Pong {
                version: PROTOCOL_VERSION,
                session_count: self.sessions.count(),
            },

            Request::Shutdown => {
                eprintln!("[daemon] shutdown requested");
                self.running = false;
                Response::ShuttingDown
            }
        }
    }

    fn handle_create(
        &mut self,
        kq: RawFd,
        shell: Option<String>,
        cols: u16,
        rows: u16,
        working_dir: Option<String>,
        terminal_id: Option<u32>,
        envs: Option<&HashMap<String, String>>,
    ) -> Response {
        let shell_str = shell.as_deref().unwrap_or("");
        match pty::create_pty(
            shell_str,
            cols,
            rows,
            working_dir.as_deref(),
            terminal_id,
            envs,
        ) {
            Ok(pty_pair) => {
                let master_fd = pty_pair.master_fd;
                let child_pid = pty_pair.child_pid;
                let session = Session::new(
                    pty_pair.master_fd,
                    pty_pair.child_pid,
                    pty_pair.ptsname.clone(),
                    WinSize { cols, rows },
                    terminal_id,
                );
                let id = session.id;

                // 阻止 PtyPair::drop 关闭 fd（已移交 session）
                std::mem::forget(pty_pair);

                // 注册 master_fd 到 kqueue
                let _ = kq_register(kq, master_fd, libc::EVFILT_READ, libc::EV_ADD);

                // 注册 EVFILT_PROC 监听子进程退出
                let proc_ev = libc::kevent {
                    ident: child_pid as usize,
                    filter: libc::EVFILT_PROC,
                    flags: libc::EV_ADD | libc::EV_ONESHOT,
                    fflags: libc::NOTE_EXIT,
                    data: 0,
                    udata: std::ptr::null_mut(),
                };
                unsafe {
                    libc::kevent(kq, &proc_ev, 1, std::ptr::null_mut(), 0, std::ptr::null());
                }

                self.sessions.add(session);
                eprintln!("[daemon] created session {id}");

                Response::Created { session_id: id }
            }
            Err(e) => Response::Error {
                message: format!("create pty failed: {e}"),
            },
        }
    }

    fn handle_attach(&mut self, kq: RawFd, client_fd: RawFd, session_id: Uuid) -> Response {
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => {
                return Response::Error {
                    message: format!("session {session_id} not found"),
                }
            }
        };

        // Deny Unix socket attach only if another Unix client is already attached.
        // WebSocket clients are handled separately and don't block Unix attach.
        if session.state == SessionState::Attached {
            return Response::AttachDeny {
                session_id,
                reason: "already attached by another local client".to_string(),
            };
        }

        if !session.is_child_alive() {
            return Response::Error {
                message: format!("session {session_id} child process is dead"),
            };
        }

        // dup(master_fd) — ETerm gets a direct copy of the PTY master fd
        let dup_fd = unsafe { libc::dup(session.master_fd) };
        if dup_fd < 0 {
            return Response::Error {
                message: format!("dup(master_fd) failed: {}", io::Error::last_os_error()),
            };
        }

        // Capture daemon-side grid snapshot before attach.
        // This reflects the terminal state the daemon has been tracking
        // while in Active/Idle. On first attach (no prior detach), it
        // gives the initial output. On reattach after crash, it gives
        // the state the daemon parsed since it re-registered master_fd.
        if let Some(ref ts) = session.terminal_state {
            let daemon_snapshot = ts.capture_snapshot();
            if daemon_snapshot.is_some() {
                session.grid_snapshot = daemon_snapshot;
            }
        }

        let cols = session.winsize.cols;
        let rows = session.winsize.rows;
        let child_pid = session.child_pid;
        let shm_name = session.shm_name.clone();
        // Write snapshot to file instead of sending via socket protocol
        // (macOS sendmsg EMSGSIZE when send buffer has pending data)
        if let Some(ref data) = session.grid_snapshot {
            let path = format!("/tmp/ptyd-snap-{}.bin", &session_id.to_string()[..8]);
            match std::fs::write(&path, data.as_bytes()) {
                Ok(_) => eprintln!(
                    "[daemon] attach {session_id}: snapshot → {path} ({} bytes)",
                    data.len()
                ),
                Err(e) => eprintln!("[daemon] attach {session_id}: snapshot write failed: {e}"),
            }
        }
        session.grid_snapshot = None;
        let grid_snapshot: Option<String> = None;
        eprintln!(
            "[daemon] attach session {session_id}: shm={shm_name}, shared_ring {} bytes",
            session.shared_ring.len()
        );

        // Unregister master_fd from kqueue — daemon sleeps while ETerm is attached
        let _ = kq_register(kq, session.master_fd, libc::EVFILT_READ, libc::EV_DELETE);

        session.attach(client_fd);

        // Stash the dup'd fd for send_attach_data
        self.pending_dup_fds.insert(session_id, dup_fd);

        eprintln!("[daemon] attach session {session_id} to fd={client_fd} (dup_fd={dup_fd}, master_fd={})", session.master_fd);

        Response::AttachReady {
            session_id,
            cols,
            rows,
            child_pid: child_pid as i32,
            shm_name,
            grid_snapshot,
        }
    }

    /// AttachReady 后发送 dup(master_fd)，ring data 由客户端直接从 shm 读取
    fn send_attach_data(&mut self, client_fd: RawFd, session_id: Uuid) -> io::Result<()> {
        let dup_fd = self
            .pending_dup_fds
            .remove(&session_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "dup_fd not found"))?;

        let raw_client_fd = match self.clients.get(&client_fd) {
            Some(c) => c.stream.as_raw_fd(),
            None => {
                unsafe {
                    libc::close(dup_fd);
                }
                return Err(io::Error::new(io::ErrorKind::NotFound, "client gone"));
            }
        };

        let send_result = fd_passing::send_fd(raw_client_fd, dup_fd);
        unsafe {
            libc::close(dup_fd);
        }
        send_result
    }

    /// Attach 失败后回滚：恢复 Active 状态，重新注册 master_fd 到 kqueue
    fn revert_attach(&mut self, kq: RawFd, session_id: Uuid) {
        // 清理 pending dup_fd
        if let Some(dup_fd) = self.pending_dup_fds.remove(&session_id) {
            unsafe {
                libc::close(dup_fd);
            }
        }
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.detach(None);
            // Re-register master_fd — daemon resumes reading
            let _ = kq_register(kq, session.master_fd, libc::EVFILT_READ, libc::EV_ADD);
        }
    }

    fn handle_detach(
        &mut self,
        kq: RawFd,
        session_id: Uuid,
        cols: u16,
        rows: u16,
        grid_snapshot: Option<String>,
    ) -> Response {
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => {
                return Response::Error {
                    message: format!("session {session_id} not found"),
                }
            }
        };

        if grid_snapshot.is_some() {
            eprintln!("[daemon] detach session {session_id}: received grid snapshot");
        }
        session.detach(grid_snapshot);
        let master_fd = session.master_fd;

        // Re-register master_fd — daemon resumes reading shell output → ring buffer
        let _ = kq_register(kq, master_fd, libc::EVFILT_READ, libc::EV_ADD);

        // Only update winsize if cols/rows are non-zero.
        // WS clients (iPhone) send 0,0 to indicate "don't change PTY dimensions"
        // — they observe at the original size and scale-to-fit on their end.
        if cols > 0 && rows > 0 {
            session.winsize = WinSize { cols, rows };
            let _ = pty::set_winsize(master_fd, cols, rows);
        }

        eprintln!("[daemon] detach session {session_id}");
        Response::Detached { session_id }
    }

    /// Baton-pass takeover: WS client requests detach of an ETerm-attached session.
    ///
    /// Instead of immediately re-registering master_fd (which would cause two readers),
    /// we send ForceDetach to ETerm, wait for DetachAck with grid snapshot, then
    /// re-register master_fd as sole reader.
    ///
    /// Returns (Response for WS client, grid_snapshot from ETerm if obtained).
    fn handle_baton_takeover(
        &mut self,
        kq: RawFd,
        session_id: Uuid,
    ) -> Response {
        let session = match self.sessions.get(&session_id) {
            Some(s) => s,
            None => {
                return Response::Error {
                    message: format!("session {session_id} not found"),
                }
            }
        };

        if session.state != SessionState::Attached {
            // Not attached by ETerm — just do normal detach (no baton needed)
            return self.handle_detach(kq, session_id, 0, 0, None);
        }

        let owner_fd = match session.owner_fd {
            Some(fd) => fd,
            None => {
                // Attached but no owner_fd — should not happen, treat as force detach
                eprintln!("[daemon] baton-takeover: session {session_id} attached but no owner_fd, force detach");
                return self.handle_detach(kq, session_id, 0, 0, None);
            }
        };

        // Step 1: Send ForceDetach push to ETerm's control socket
        let push = Push::ForceDetach { session_id };
        let push_bytes = protocol::encode_message(&push);

        let send_ok = if let Some(client) = self.clients.get_mut(&owner_fd) {
            client.stream.set_nonblocking(false).ok();
            let _ = client.stream.set_write_timeout(Some(Duration::from_secs(2)));
            let result = client.stream.write_all(&push_bytes);
            client.stream.set_nonblocking(true).ok();
            result.is_ok()
        } else {
            false
        };

        if !send_ok {
            eprintln!(
                "[daemon] baton-takeover: failed to send ForceDetach to fd={owner_fd}, force detach"
            );
            // ETerm's control socket is broken — treat as crash
            return self.force_complete_takeover(kq, session_id, None);
        }

        eprintln!("[daemon] baton-takeover: sent ForceDetach to ETerm fd={owner_fd}, waiting for DetachAck");

        // Step 2: Wait for DetachAck with timeout
        let grid_snapshot = self.wait_for_detach_ack(owner_fd, session_id);

        // Step 3: Complete the takeover
        self.force_complete_takeover(kq, session_id, grid_snapshot)
    }

    /// Wait for DetachAck from ETerm on the control socket, with timeout.
    ///
    /// Returns the grid_snapshot if ETerm responded, or None on timeout/error.
    fn wait_for_detach_ack(
        &mut self,
        owner_fd: RawFd,
        session_id: Uuid,
    ) -> Option<String> {
        let client = match self.clients.get_mut(&owner_fd) {
            Some(c) => c,
            None => return None,
        };

        // Set blocking with timeout for the read
        client.stream.set_nonblocking(false).ok();
        let _ = client.stream.set_read_timeout(Some(BATON_PASS_TIMEOUT));

        let mut tmp = [0u8; 65536];
        let deadline = Instant::now() + BATON_PASS_TIMEOUT;

        loop {
            if Instant::now() >= deadline {
                eprintln!(
                    "[daemon] baton-takeover: DetachAck timeout for session {session_id}"
                );
                client.stream.set_nonblocking(true).ok();
                return None;
            }

            match client.stream.read(&mut tmp) {
                Ok(0) => {
                    eprintln!("[daemon] baton-takeover: ETerm closed socket during wait");
                    client.stream.set_nonblocking(true).ok();
                    return None;
                }
                Ok(n) => {
                    client.read_buf.extend_from_slice(&tmp[..n]);
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut
                    {
                        eprintln!(
                            "[daemon] baton-takeover: DetachAck timed out for session {session_id}"
                        );
                    } else {
                        eprintln!(
                            "[daemon] baton-takeover: read error waiting for DetachAck: {e}"
                        );
                    }
                    client.stream.set_nonblocking(true).ok();
                    return None;
                }
            }

            // Try to decode PushAck from the buffer
            let buf_snapshot = client.read_buf.clone();
            if let Some((result, consumed)) =
                protocol::try_decode_message::<PushAck>(&buf_snapshot)
            {
                client.read_buf.drain(..consumed);
                client.stream.set_nonblocking(true).ok();

                match result {
                    Ok(PushAck::DetachAck {
                        session_id: ack_sid,
                        grid_snapshot,
                    }) => {
                        if ack_sid == session_id {
                            eprintln!(
                                "[daemon] baton-takeover: received DetachAck for session {session_id} (snapshot={})",
                                grid_snapshot.is_some()
                            );
                            return grid_snapshot;
                        }
                        // Wrong session — unexpected but log and return None
                        eprintln!(
                            "[daemon] baton-takeover: DetachAck for wrong session: expected {session_id}, got {ack_sid}"
                        );
                        return None;
                    }
                    Ok(other) => {
                        eprintln!(
                            "[daemon] baton-takeover: unexpected PushAck variant: {:?}", other
                        );
                        return None;
                    }
                    Err(e) => {
                        // Might be a regular Request instead of PushAck — try decoding as Request
                        // If ETerm sends a normal Detach request instead of PushAck, handle gracefully
                        eprintln!(
                            "[daemon] baton-takeover: PushAck decode error: {e}, ignoring"
                        );
                        return None;
                    }
                }
            }
            // Not enough data yet — continue reading
        }
    }

    /// Complete a baton-pass takeover after ForceDetach (whether we got DetachAck or timed out).
    ///
    /// Re-registers master_fd so daemon becomes sole reader, stores snapshot if provided.
    fn force_complete_takeover(
        &mut self,
        kq: RawFd,
        session_id: Uuid,
        grid_snapshot: Option<String>,
    ) -> Response {
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => {
                return Response::Error {
                    message: format!("session {session_id} not found"),
                }
            }
        };

        // Store the grid snapshot from ETerm (the baton).
        // Use baton_detach to preserve previous_owner_fd for later baton-release.
        session.baton_detach(grid_snapshot);
        let master_fd = session.master_fd;

        // Re-register master_fd — daemon is now the sole reader
        let _ = kq_register(kq, master_fd, libc::EVFILT_READ, libc::EV_ADD);

        eprintln!("[daemon] baton-takeover: completed for session {session_id}");
        Response::Detached { session_id }
    }

    /// Baton-pass release: WS client disconnects / detaches, offer session back to ETerm.
    ///
    /// The daemon has been reading master_fd while the WS client was attached.
    /// Capture the daemon's grid snapshot and push ResumeAttach to ETerm.
    fn handle_baton_release(
        &mut self,
        kq: RawFd,
        session_id: Uuid,
    ) -> Response {
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => {
                return Response::Error {
                    message: format!("session {session_id} not found"),
                }
            }
        };

        // Only do baton release if session has a previous owner that we can push to.
        // After a takeover, owner_fd is cleared. We need to find an ETerm client
        // that was previously attached. For now, look for the original owner from
        // the session's context.
        //
        // In the current design, after baton takeover the owner_fd is None.
        // We need to store the original ETerm client_fd to push ResumeAttach.
        // This is tracked by `previous_owner_fd` added to the Session struct.
        let prev_owner_fd = match session.previous_owner_fd {
            Some(fd) => fd,
            None => {
                eprintln!(
                    "[daemon] baton-release: no previous owner for session {session_id}, normal detach"
                );
                return self.handle_detach(kq, session_id, 0, 0, None);
            }
        };

        // Check if the previous owner is still connected
        if !self.clients.contains_key(&prev_owner_fd) {
            eprintln!(
                "[daemon] baton-release: previous owner fd={prev_owner_fd} disconnected"
            );
            session.previous_owner_fd = None;
            return self.handle_detach(kq, session_id, 0, 0, None);
        }

        // Capture daemon's grid snapshot (daemon has been reading, its state is current)
        let grid_snapshot = session
            .terminal_state
            .as_ref()
            .and_then(|ts| ts.capture_snapshot());

        // dup(master_fd) for ETerm
        let dup_fd = unsafe { libc::dup(session.master_fd) };
        if dup_fd < 0 {
            eprintln!(
                "[daemon] baton-release: dup(master_fd) failed: {}",
                io::Error::last_os_error()
            );
            return self.handle_detach(kq, session_id, 0, 0, None);
        }

        let cols = session.winsize.cols;
        let rows = session.winsize.rows;
        let child_pid = session.child_pid;
        let shm_name = session.shm_name.clone();

        // Send ResumeAttach push to ETerm
        let push = Push::ResumeAttach {
            session_id,
            cols,
            rows,
            child_pid: child_pid as i32,
            shm_name: shm_name.clone(),
            grid_snapshot: grid_snapshot.clone(),
        };
        let push_bytes = protocol::encode_message(&push);

        let send_ok = if let Some(client) = self.clients.get_mut(&prev_owner_fd) {
            client.stream.set_nonblocking(false).ok();
            let _ = client.stream.set_write_timeout(Some(Duration::from_secs(2)));
            let result = client.stream.write_all(&push_bytes);
            if result.is_ok() {
                // Also send the dup_fd via SCM_RIGHTS
                let fd_result = fd_passing::send_fd(client.stream.as_raw_fd(), dup_fd);
                client.stream.set_nonblocking(true).ok();
                unsafe { libc::close(dup_fd); }
                fd_result.is_ok()
            } else {
                client.stream.set_nonblocking(true).ok();
                unsafe { libc::close(dup_fd); }
                false
            }
        } else {
            unsafe { libc::close(dup_fd); }
            false
        };

        if !send_ok {
            eprintln!(
                "[daemon] baton-release: failed to send ResumeAttach to fd={prev_owner_fd}"
            );
            session.previous_owner_fd = None;
            return self.handle_detach(kq, session_id, 0, 0, grid_snapshot);
        }

        // Unregister master_fd from kqueue — ETerm is now the reader again
        let _ = kq_register(kq, session.master_fd, libc::EVFILT_READ, libc::EV_DELETE);

        // Transition to Attached state with the previous owner
        session.attach(prev_owner_fd);
        // Clear previous_owner_fd since we've restored the attach
        session.previous_owner_fd = None;

        // Store dup_fd in pending (not needed since we already sent it, but keep consistent)
        // The fd was already sent above, no need for pending_dup_fds.

        eprintln!(
            "[daemon] baton-release: session {session_id} returned to ETerm fd={prev_owner_fd}"
        );
        Response::Detached { session_id }
    }

    fn handle_list(&self) -> Response {
        let now = Instant::now();
        let sessions: Vec<SessionInfo> = self
            .sessions
            .list()
            .iter()
            .map(|s| SessionInfo {
                id: s.id,
                state: format!("{:?}", s.state),
                child_pid: s.child_pid,
                cols: s.winsize.cols,
                rows: s.winsize.rows,
                ptsname: s.ptsname.clone(),
                child_alive: s.is_child_alive(),
                ring_buffer_bytes: s.shared_ring.total_written(),
                created_secs_ago: now.duration_since(s.created_at).as_secs(),
                last_active_secs_ago: now.duration_since(s.last_active).as_secs(),
                terminal_id: s.terminal_id,
            })
            .collect();

        Response::SessionList { sessions }
    }

    fn handle_kill(&mut self, kq: RawFd, session_id: Uuid) -> Response {
        match self.sessions.remove(&session_id) {
            Some(mut session) => {
                let _ = kq_register(kq, session.master_fd, libc::EVFILT_READ, libc::EV_DELETE);
                // Notify WS subscribers
                if let Some(ref ws_shared) = self.ws_shared {
                    let mut state = ws_shared.lock().unwrap();
                    state.notify_session_ended(&session_id);
                }
                session.cleanup();
                session.master_fd = -1;
                eprintln!("[daemon] killed session {session_id}");
                Response::Killed { session_id }
            }
            None => Response::Error {
                message: format!("session {session_id} not found"),
            },
        }
    }

    fn handle_winsize_update(&mut self, session_id: Uuid, cols: u16, rows: u16) -> Response {
        match self.sessions.get_mut(&session_id) {
            Some(session) => {
                let size_changed = session.winsize.cols != cols || session.winsize.rows != rows;
                session.winsize = WinSize { cols, rows };
                if let Some(ref mut ts) = session.terminal_state {
                    ts.resize(cols, rows);
                }
                // Attached: ETerm does ioctl directly on its dup(master_fd), daemon just records
                // Detached: daemon is responsible for ioctl + SIGWINCH
                if session.state != SessionState::Attached {
                    let _ = pty::set_winsize(session.master_fd, cols, rows);
                    // ioctl only sends SIGWINCH if size changed. If same size,
                    // explicitly signal so programs redraw (e.g. after WS takeover).
                    if !size_changed {
                        unsafe { libc::kill(session.child_pid, libc::SIGWINCH); }
                    }
                }
                Response::WinsizeUpdated { session_id }
            }
            None => Response::Error {
                message: format!("session {session_id} not found"),
            },
        }
    }

    /// 子进程退出事件处理（EVFILT_PROC + NOTE_EXIT）
    fn handle_child_exit(&mut self, kq: RawFd, pid: libc::pid_t) {
        // waitpid 回收 zombie
        let mut status: libc::c_int = 0;
        unsafe {
            libc::waitpid(pid, &mut status, libc::WNOHANG);
        }

        // 找到对应 session 并清理
        let session_info = self
            .sessions
            .find_by_child_pid(pid)
            .map(|s| (s.id, s.master_fd));

        if let Some((id, master_fd)) = session_info {
            eprintln!("[daemon] child pid={pid} exited, cleaning session {id}");
            let _ = kq_register(kq, master_fd, libc::EVFILT_READ, libc::EV_DELETE);
            // Notify WS subscribers
            if let Some(ref ws_shared) = self.ws_shared {
                let mut state = ws_shared.lock().unwrap();
                state.notify_session_ended(&id);
            }
            self.sessions.remove(&id);
        }
    }

    /// 处理 session 的 PTY 输出（master_fd 可读）
    ///
    /// fd passing 模式：Attached 时 master_fd 已从 kqueue 注销，不会到这里。
    /// 只在 Active/Idle 时读取 master_fd → 写入 ring_buffer。
    fn handle_session_output(&mut self, kq: RawFd, fd: RawFd) {
        let session_id = self.sessions.find_by_master_fd(fd).map(|s| s.id);

        let session_id = match session_id {
            Some(id) => id,
            None => return,
        };

        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => return,
        };

        // 防御：Attached 时 master_fd 应已从 kqueue 注销，但同一批事件可能残留
        if session.state == SessionState::Attached {
            return;
        }

        let mut buf = [0u8; 8192];
        match unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) } {
            n if n > 0 => {
                let data = &buf[..n as usize];
                session.shared_ring.write(data);
                if let Some(ref mut ts) = session.terminal_state {
                    ts.feed(data);
                }
                session.last_active = Instant::now();
                // Broadcast to WebSocket subscribers
                if let Some(ref ws_shared) = self.ws_shared {
                    let state = ws_shared.lock().unwrap();
                    state.broadcast(&session_id, data);
                }
                if session.state == SessionState::Idle {
                    session.promote_to_active();
                }
            }
            0 => {
                eprintln!("[daemon] session {session_id} EOF on master_fd, removing");
                let _ = kq_register(kq, fd, libc::EVFILT_READ, libc::EV_DELETE);
                let _ = session;
                self.sessions.remove(&session_id);
            }
            _ => {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::WouldBlock {
                    eprintln!("[daemon] session {session_id} read error: {err}");
                }
            }
        }
    }

    /// Drain all pending WebSocket commands (non-blocking)
    fn drain_ws_commands(&mut self, kq: RawFd) {
        // Collect commands first to avoid borrow conflict
        let mut commands = Vec::new();
        let mut disconnected = false;

        if let Some(ref rx) = self.ws_cmd_rx {
            loop {
                match rx.try_recv() {
                    Ok(cmd) => commands.push(cmd),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        eprintln!("[daemon] ws command channel disconnected");
                        disconnected = true;
                        break;
                    }
                }
            }
        }

        if disconnected {
            self.ws_cmd_rx = None;
        }

        for cmd in commands {
            self.handle_ws_command(kq, cmd);
        }
    }

    /// Handle a single WebSocket command
    fn handle_ws_command(&mut self, kq: RawFd, cmd: WsCommand) {
        match cmd {
            WsCommand::Request {
                client_id,
                request,
                reply_tx,
            } => {
                let response = match request {
                    Request::Attach { session_id } => {
                        self.handle_ws_attach(kq, client_id, session_id)
                    }
                    // WS Detach on an ETerm-attached session → baton-pass takeover
                    Request::Detach { session_id, .. } => {
                        self.handle_ws_detach(kq, client_id, session_id)
                    }
                    other => {
                        // Use a synthetic fd=-1 for WS clients (they don't have a Unix fd)
                        self.handle_request(kq, -1, other)
                    }
                };
                let _ = reply_tx.send(response);
            }
            WsCommand::Disconnected { client_id } => {
                self.handle_ws_client_disconnect(kq, client_id);
            }
            WsCommand::Input { session_id, data } => {
                self.handle_ws_input(session_id, &data);
            }
        }
    }

    /// Handle WebSocket Detach — routes to the appropriate handler:
    ///
    /// 1. If ETerm is currently attached (takeover): baton-pass takeover
    /// 2. If ETerm was previously attached and is waiting (release): baton-pass release
    /// 3. Otherwise: normal detach
    fn handle_ws_detach(&mut self, kq: RawFd, _client_id: u64, session_id: Uuid) -> Response {
        let (is_eterm_attached, has_previous_owner) = self
            .sessions
            .get(&session_id)
            .map_or((false, false), |s| {
                (
                    s.state == SessionState::Attached,
                    s.previous_owner_fd.is_some(),
                )
            });

        if is_eterm_attached {
            // ETerm owns this session — initiate baton-pass takeover
            eprintln!(
                "[daemon] ws-detach: session {session_id} is ETerm-attached, initiating baton-pass takeover"
            );
            self.handle_baton_takeover(kq, session_id)
        } else if has_previous_owner {
            // iPhone is releasing — return session to ETerm via baton-pass release
            eprintln!(
                "[daemon] ws-detach: session {session_id} has previous ETerm owner, baton-pass release"
            );
            self.handle_baton_release(kq, session_id)
        } else {
            // No ETerm involved — normal detach
            self.handle_detach(kq, session_id, 0, 0, None)
        }
    }

    /// Handle WebSocket Attach — no fd-passing, daemon keeps reading master_fd
    ///
    /// Unlike Unix socket attach, the daemon stays active and proxies PTY I/O.
    /// Multiple WS clients and one Unix client can be attached simultaneously.
    fn handle_ws_attach(&mut self, kq: RawFd, client_id: u64, session_id: Uuid) -> Response {
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => {
                return Response::Error {
                    message: format!("session {session_id} not found"),
                }
            }
        };

        if !session.is_child_alive() {
            return Response::Error {
                message: format!("session {session_id} child process is dead"),
            };
        }

        // Capture daemon-side grid snapshot for the WS client
        if let Some(ref ts) = session.terminal_state {
            let daemon_snapshot = ts.capture_snapshot();
            if daemon_snapshot.is_some() {
                session.grid_snapshot = daemon_snapshot;
            }
        }

        let cols = session.winsize.cols;
        let rows = session.winsize.rows;
        let child_pid = session.child_pid;
        let shm_name = session.shm_name.clone();

        // For WS clients, include ring buffer data in the response as initial replay data.
        // (WS clients can't access shared memory across the network.)
        let ring_data = session.shared_ring.dump();
        let grid_snapshot = if !ring_data.is_empty() {
            Some(base64::engine::general_purpose::STANDARD.encode(&ring_data))
        } else {
            session.grid_snapshot.take()
        };

        // If the session is currently in Active/Idle, ensure master_fd is registered in kqueue
        // so the daemon can read output and broadcast to WS subscribers.
        if session.state != SessionState::Attached {
            let _ = kq_register(kq, session.master_fd, libc::EVFILT_READ, libc::EV_ADD);
        }
        // If the session is Attached by a Unix client, we still accept the WS attach.
        // The Unix client is reading the master_fd directly; the daemon will NOT be reading
        // master_fd (it's unregistered from kqueue). So the WS client won't get real-time
        // output until the Unix client detaches. This is a known limitation that could be
        // addressed in a future multi-reader architecture.

        // Track the WS client attachment (subscription is set up by the ws_server module)
        session.ws_client_count += 1;

        eprintln!(
            "[daemon] ws-attach session {session_id} by ws-client {client_id} \
             (ws_clients={}, unix_attached={})",
            session.ws_client_count,
            session.state == SessionState::Attached
        );

        Response::AttachReady {
            session_id,
            cols,
            rows,
            child_pid: child_pid as i32,
            shm_name,
            grid_snapshot,
        }
    }

    /// Handle input data from a WebSocket client → write to PTY master_fd
    fn handle_ws_input(&self, session_id: Uuid, data: &[u8]) {
        if let Some(session) = self.sessions.get(&session_id) {
            let fd = session.master_fd;
            let mut offset = 0;
            while offset < data.len() {
                let n = unsafe {
                    libc::write(
                        fd,
                        data[offset..].as_ptr() as *const libc::c_void,
                        data.len() - offset,
                    )
                };
                if n <= 0 {
                    break;
                }
                offset += n as usize;
            }
        }
    }

    /// Handle WebSocket client disconnect — clean up session attachment.
    ///
    /// If this was the last WS client and the session has a previous ETerm owner,
    /// trigger baton-release to return the session to ETerm.
    fn handle_ws_client_disconnect(&mut self, kq: RawFd, client_id: u64) {
        // Find which session this client was attached to
        let session_id = self
            .ws_shared
            .as_ref()
            .and_then(|s| s.lock().unwrap().client_sessions.get(&client_id).copied());

        if let Some(session_id) = session_id {
            let should_release = if let Some(session) = self.sessions.get_mut(&session_id) {
                if session.ws_client_count > 0 {
                    session.ws_client_count -= 1;
                }
                eprintln!(
                    "[daemon] ws-client {client_id} disconnected from session {session_id} \
                     (remaining ws_clients={})",
                    session.ws_client_count
                );
                // Trigger baton-release if this was the last WS client and
                // there's a previous ETerm owner waiting
                session.ws_client_count == 0 && session.previous_owner_fd.is_some()
            } else {
                false
            };

            if should_release {
                eprintln!(
                    "[daemon] ws-client {client_id}: last WS client left session {session_id}, baton-release to ETerm"
                );
                self.handle_baton_release(kq, session_id);
            }
            // Unsubscribe is handled by the ws_server module
        }
    }

    fn cleanup(&mut self) {
        // Notify all WS subscribers that sessions are ending
        if let Some(ref ws_shared) = self.ws_shared {
            let mut state = ws_shared.lock().unwrap();
            let session_ids: Vec<Uuid> = state.subscribers.keys().copied().collect();
            for id in session_ids {
                state.notify_session_ended(&id);
            }
        }

        // 清理所有存活 session（kill 子进程，关 fd，unlink shm）
        let ids: Vec<Uuid> = self.sessions.list().iter().map(|s| s.id).collect();
        for id in ids {
            if let Some(mut session) = self.sessions.remove(&id) {
                eprintln!("[daemon] cleanup: killing session {id}");
                session.cleanup();
                session.master_fd = -1;
            }
        }
        // 关闭所有残留的 pending dup_fd
        for (_, dup_fd) in self.pending_dup_fds.drain() {
            unsafe {
                libc::close(dup_fd);
            }
        }
        // Close self-pipe
        if let Some((read_fd, write_fd)) = self.ws_wakeup_pipe.take() {
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
        }
        let _ = std::fs::remove_file(&self.socket_path);
        eprintln!("[daemon] cleanup done");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cleanup();
    }
}
