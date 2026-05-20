//! Unix domain socket 控制面 server
//!
//! 单线程 kqueue 事件循环：
//! - 监听控制 socket 新连接
//! - 处理客户端请求（Create/Attach/Detach/List/Kill/WinsizeUpdate）
//! - fd passing 直连模式：Attach 时 dup(master_fd) 传给 ETerm，daemon 休眠
//! - Detach/崩溃时 daemon 重新注册 master_fd，读输出写 ring_buffer
//! - 监听 attached session 的 owner socket 断开（崩溃检测）

use crate::fd_passing;
use crate::protocol::{self, Request, Response, SessionInfo, PROTOCOL_VERSION};
use crate::pty;
use crate::session::{Session, SessionManager, SessionState, WinSize};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::RawFd;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Idle timeout: daemon exits after this duration with 0 sessions and 0 clients
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

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

        Ok(Self {
            socket_path: socket_path.to_owned(),
            listener,
            sessions: SessionManager::new(),
            clients: HashMap::new(),
            pending_dup_fds: HashMap::new(),
            running: true,
            idle_since: Some(Instant::now()),
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
                eprintln!("[daemon] reaped dead session {id}");
            }

            // Idle auto-exit：0 sessions + 0 clients 持续 IDLE_TIMEOUT 后退出
            if self.sessions.count() == 0 && self.clients.is_empty() {
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

                if ev.filter == libc::EVFILT_SIGNAL {
                    let sig = ev.ident as i32;
                    eprintln!("[daemon] received signal {sig}, shutting down");
                    self.running = false;
                    break;
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

                    // 发送响应
                    let resp_bytes = protocol::encode_message(&response);
                    if let Some(client) = self.clients.get_mut(&fd) {
                        let _ = client.stream.write_all(&resp_bytes);
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
                s.detach();
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
            } => self.handle_detach(kq, session_id, cols, rows),

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

        if session.state == SessionState::Attached {
            return Response::AttachDeny {
                session_id,
                reason: "already attached".to_string(),
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

        let cols = session.winsize.cols;
        let rows = session.winsize.rows;
        let child_pid = session.child_pid;
        let shm_name = session.shm_name.clone();
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
            session.detach();
            // Re-register master_fd — daemon resumes reading
            let _ = kq_register(kq, session.master_fd, libc::EVFILT_READ, libc::EV_ADD);
        }
    }

    fn handle_detach(&mut self, kq: RawFd, session_id: Uuid, cols: u16, rows: u16) -> Response {
        let session = match self.sessions.get_mut(&session_id) {
            Some(s) => s,
            None => {
                return Response::Error {
                    message: format!("session {session_id} not found"),
                }
            }
        };

        session.detach();
        session.winsize = WinSize { cols, rows };
        let master_fd = session.master_fd;

        // Re-register master_fd — daemon resumes reading shell output → ring buffer
        let _ = kq_register(kq, master_fd, libc::EVFILT_READ, libc::EV_ADD);

        // Detached: daemon is responsible for ioctl
        let _ = pty::set_winsize(master_fd, cols, rows);

        eprintln!("[daemon] detach session {session_id}");
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
                child_pid: s.child_pid as i32,
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
                session.winsize = WinSize { cols, rows };
                // Attached: ETerm does ioctl directly on its dup(master_fd), daemon just records
                // Detached: daemon is responsible for ioctl
                if session.state != SessionState::Attached {
                    let _ = pty::set_winsize(session.master_fd, cols, rows);
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
                session.last_active = Instant::now();
                // eprintln!("[daemon] session {session_id} read {n} bytes from master_fd, shared_ring now {} bytes", session.shared_ring.len());
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

    fn cleanup(&mut self) {
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
        let _ = std::fs::remove_file(&self.socket_path);
        eprintln!("[daemon] cleanup done");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.cleanup();
    }
}
