//! pty-daemon 集成测试
//!
//! 覆盖：daemon 启动、session CRUD、attach/detach、崩溃恢复、
//! ring buffer 回放、kill、shutdown、生命周期管理

use pty_daemon::fd_passing;
use pty_daemon::protocol::{self, Request, Response};
use pty_daemon::server::Server;
use pty_daemon::shared_ring::SharedRingBuffer;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

// ===========================================================================
// Test harness
// ===========================================================================

/// RAII test daemon — 启动后台线程运行 Server，Drop 时自动 shutdown + join + 清理 socket
struct TestDaemon {
    sock: PathBuf,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestDaemon {
    /// 启动 daemon，阻塞直到 socket 就绪
    fn start(name: &str) -> Self {
        let sock = PathBuf::from(format!(
            "/tmp/pty-daemon-test-{name}-{}.sock",
            std::process::id()
        ));
        // 清理残留
        let _ = std::fs::remove_file(&sock);

        let sock2 = sock.clone();
        let handle = thread::spawn(move || {
            let mut server = Server::new(&sock2).expect("start daemon");
            let _ = server.run();
        });

        // 等 socket 出现
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if sock.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(sock.exists(), "daemon socket not created within 3s");

        Self {
            sock,
            handle: Some(handle),
        }
    }

    fn sock(&self) -> &PathBuf {
        &self.sock
    }

    /// 发送 Shutdown 并等待线程退出
    fn shutdown(mut self) {
        self.do_shutdown();
    }

    fn do_shutdown(&mut self) {
        if self.sock.exists() {
            let _ = Client::oneshot(&self.sock, &Request::Shutdown);
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&self.sock);
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.do_shutdown();
    }
}

// ===========================================================================
// Protocol client helpers
// ===========================================================================

struct Client;

impl Client {
    /// 单次请求（内部建连 → 发送 → 读响应 → 断开）
    fn oneshot(sock: &PathBuf, req: &Request) -> Response {
        let mut stream = UnixStream::connect(sock).expect("connect to daemon");
        Self::send(&mut stream, req)
    }

    /// 在已有连接上发送请求并读响应
    fn send(stream: &mut UnixStream, req: &Request) -> Response {
        let encoded = protocol::encode_message(req);
        stream.write_all(&encoded).expect("write request");

        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(
                Instant::now() < deadline,
                "timeout waiting for daemon response"
            );
            let n = stream.read(&mut tmp).expect("read response");
            assert!(n > 0, "daemon closed connection unexpectedly");
            buf.extend_from_slice(&tmp[..n]);
            if let Some((result, _)) = protocol::try_decode_message::<Response>(&buf) {
                return result.expect("parse response");
            }
        }
    }

    fn ping(sock: &PathBuf) -> (u16, usize) {
        match Self::oneshot(sock, &Request::Ping) {
            Response::Pong {
                version,
                session_count,
            } => (version, session_count),
            other => panic!("expected Pong, got: {other:?}"),
        }
    }

    fn session_count(sock: &PathBuf) -> usize {
        Self::ping(sock).1
    }

    fn create(sock: &PathBuf) -> uuid::Uuid {
        match Self::oneshot(
            sock,
            &Request::Create {
                shell: None,
                cols: 80,
                rows: 24,
                working_dir: None,
                terminal_id: None,
                envs: None,
            },
        ) {
            Response::Created { session_id } => session_id,
            other => panic!("expected Created, got: {other:?}"),
        }
    }

    fn session_state(sock: &PathBuf, id: uuid::Uuid) -> Option<String> {
        match Self::oneshot(sock, &Request::List) {
            Response::SessionList { sessions } => sessions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.state.clone()),
            _ => None,
        }
    }
}

// ===========================================================================
// Attached session — RAII wrapper for attach lifecycle
// ===========================================================================

/// Attached session 句柄：持有 control stream + dup(master_fd)，Drop 时自动关闭
struct AttachedSession {
    /// 持有 control stream 的 ownership，Drop 时自动关闭触发 daemon 崩溃检测
    _control: UnixStream,
    pty_fd: RawFd,
    shm_name: String,
}

impl AttachedSession {
    fn attach(sock: &PathBuf, session_id: uuid::Uuid) -> Self {
        let mut stream = UnixStream::connect(sock).expect("connect for attach");
        let resp = Client::send(&mut stream, &Request::Attach { session_id });
        match resp {
            Response::AttachReady { shm_name, .. } => {
                let pty_fd = fd_passing::recv_fd(stream.as_raw_fd()).expect("recv_fd");
                Self {
                    _control: stream,
                    pty_fd,
                    shm_name,
                }
            }
            other => panic!("expected AttachReady, got: {other:?}"),
        }
    }

    /// 向 PTY 写入
    fn write(&self, data: &[u8]) {
        let ret = unsafe { libc::write(self.pty_fd, data.as_ptr() as *const _, data.len()) };
        assert!(
            ret > 0,
            "pty write failed: {}",
            std::io::Error::last_os_error()
        );
    }

    /// 从 PTY 读取直到找到 marker 或超时
    fn read_until(&self, marker: &str, timeout: Duration) -> String {
        let start = Instant::now();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];

        while start.elapsed() < timeout {
            let mut pfd = libc::pollfd {
                fd: self.pty_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let remaining_ms = timeout.saturating_sub(start.elapsed()).as_millis().min(100) as i32;
            if remaining_ms <= 0 {
                break;
            }
            let ret = unsafe { libc::poll(&mut pfd, 1, remaining_ms) };
            if ret > 0 && pfd.revents & libc::POLLIN != 0 {
                let n = unsafe { libc::read(self.pty_fd, tmp.as_mut_ptr() as *mut _, tmp.len()) };
                if n > 0 {
                    buf.extend_from_slice(&tmp[..n as usize]);
                    let s = String::from_utf8_lossy(&buf);
                    if s.contains(marker) {
                        return s.to_string();
                    }
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    /// 主动 detach（发 Detach 请求，关闭 pty_fd）
    fn detach(self, sock: &PathBuf, session_id: uuid::Uuid) {
        Client::oneshot(
            sock,
            &Request::Detach {
                session_id,
                cols: 80,
                rows: 24,
                grid_snapshot: None,
            },
        );
        // Drop 会关闭 pty_fd 和 control stream
    }

    /// 模拟崩溃：直接 drop（不发 Detach），daemon 通过 EV_EOF 检测
    fn simulate_crash(self) {
        // Drop 关闭 fd，daemon 检测到 owner disconnect
    }

    fn open_ring(&self) -> SharedRingBuffer {
        SharedRingBuffer::open(&self.shm_name).expect("open shared ring buffer")
    }
}

impl Drop for AttachedSession {
    fn drop(&mut self) {
        if self.pty_fd >= 0 {
            unsafe {
                libc::close(self.pty_fd);
            }
            self.pty_fd = -1;
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[test]
fn test_ping() {
    let daemon = TestDaemon::start("ping");
    let (version, count) = Client::ping(daemon.sock());
    assert_eq!(version, protocol::PROTOCOL_VERSION);
    assert_eq!(count, 0);
}

#[test]
fn test_create_session() {
    let daemon = TestDaemon::start("create");
    let id = Client::create(daemon.sock());
    assert_eq!(Client::session_count(daemon.sock()), 1);
    assert_eq!(
        Client::session_state(daemon.sock(), id),
        Some("Active".into())
    );
}

#[test]
fn test_attach_execute_output() {
    let daemon = TestDaemon::start("attach");
    let id = Client::create(daemon.sock());
    let session = AttachedSession::attach(daemon.sock(), id);

    assert_eq!(
        Client::session_state(daemon.sock(), id),
        Some("Attached".into())
    );

    session.write(b"echo MARKER_TEST_12345\n");
    let output = session.read_until("MARKER_TEST_12345", Duration::from_secs(3));
    assert!(output.contains("MARKER_TEST_12345"), "got: {output}");
}

#[test]
fn test_detach_session_survives() {
    let daemon = TestDaemon::start("detach");
    let id = Client::create(daemon.sock());
    let session = AttachedSession::attach(daemon.sock(), id);
    session.detach(daemon.sock(), id);

    thread::sleep(Duration::from_millis(500));
    assert_eq!(Client::session_count(daemon.sock()), 1);
    assert_eq!(
        Client::session_state(daemon.sock(), id),
        Some("Active".into())
    );
}

#[test]
fn test_reattach_ring_buffer_replay() {
    let daemon = TestDaemon::start("reattach");
    let id = Client::create(daemon.sock());

    // 等 shell 启动
    thread::sleep(Duration::from_secs(1));

    // 第一次 attach：执行命令，写入 ring buffer（模拟 ETerm 行为）
    let session = AttachedSession::attach(daemon.sock(), id);
    let ring = session.open_ring();
    let shm_name = session.shm_name.clone();

    session.write(b"echo FIRST_MARKER_AAA\n");
    let output = session.read_until("FIRST_MARKER_AAA", Duration::from_secs(3));
    ring.write(output.as_bytes());

    session.detach(daemon.sock(), id);
    thread::sleep(Duration::from_secs(1));

    // Reattach：ring buffer 应包含历史
    let session2 = AttachedSession::attach(daemon.sock(), id);
    assert_eq!(session2.shm_name, shm_name, "same shm across reattach");

    let ring2 = session2.open_ring();
    let ring_data = ring2.dump();
    let ring_str = String::from_utf8_lossy(&ring_data);
    assert!(
        ring_str.contains("FIRST_MARKER_AAA"),
        "ring should contain first marker, got: {ring_str}"
    );

    // 新命令也正常
    session2.write(b"echo SECOND_MARKER_BBB\n");
    let output2 = session2.read_until("SECOND_MARKER_BBB", Duration::from_secs(3));
    assert!(output2.contains("SECOND_MARKER_BBB"), "got: {output2}");
}

#[test]
fn test_crash_detach_daemon_takeover() {
    let daemon = TestDaemon::start("crash");
    let id = Client::create(daemon.sock());
    let session = AttachedSession::attach(daemon.sock(), id);
    assert_eq!(
        Client::session_state(daemon.sock(), id),
        Some("Attached".into())
    );

    // 模拟崩溃：直接丢弃，不发 Detach
    session.simulate_crash();

    // daemon 检测到 EV_EOF，自动回滚到 Active
    thread::sleep(Duration::from_secs(1));
    assert_eq!(
        Client::session_state(daemon.sock(), id),
        Some("Active".into())
    );

    // 崩溃后 reattach 正常工作
    let session2 = AttachedSession::attach(daemon.sock(), id);
    session2.write(b"echo AFTER_CRASH_OK\n");
    let output = session2.read_until("AFTER_CRASH_OK", Duration::from_secs(3));
    assert!(output.contains("AFTER_CRASH_OK"), "got: {output}");
}

#[test]
fn test_kill_session() {
    let daemon = TestDaemon::start("kill");
    let id = Client::create(daemon.sock());
    assert_eq!(Client::session_count(daemon.sock()), 1);

    let resp = Client::oneshot(daemon.sock(), &Request::Kill { session_id: id });
    assert!(matches!(resp, Response::Killed { .. }));
    assert_eq!(Client::session_count(daemon.sock()), 0);
}

#[test]
fn test_shutdown() {
    let daemon = TestDaemon::start("shutdown");
    let sock = daemon.sock().clone();
    Client::create(&sock);

    // 发送 Shutdown，daemon 应回复后退出
    let resp = Client::oneshot(&sock, &Request::Shutdown);
    assert!(matches!(resp, Response::ShuttingDown));

    daemon.shutdown();
    assert!(!sock.exists(), "socket should be removed after shutdown");
}

#[test]
fn test_shutdown_cleans_all_sessions() {
    let daemon = TestDaemon::start("shutdown-clean");
    let sock = daemon.sock().clone();
    Client::create(&sock);
    Client::create(&sock);
    Client::create(&sock);
    assert_eq!(Client::session_count(&sock), 3);

    daemon.shutdown();
    assert!(!sock.exists(), "socket should be removed");
}

#[test]
fn test_multiple_sessions() {
    let daemon = TestDaemon::start("multi");
    let id1 = Client::create(daemon.sock());
    let id2 = Client::create(daemon.sock());
    assert_eq!(Client::session_count(daemon.sock()), 2);

    // Kill 一个，另一个不受影响
    Client::oneshot(daemon.sock(), &Request::Kill { session_id: id1 });
    assert_eq!(Client::session_count(daemon.sock()), 1);
    assert_eq!(
        Client::session_state(daemon.sock(), id2),
        Some("Active".into())
    );
    assert_eq!(Client::session_state(daemon.sock(), id1), None);
}

#[test]
fn test_attach_deny_already_attached() {
    let daemon = TestDaemon::start("deny");
    let id = Client::create(daemon.sock());
    let _session = AttachedSession::attach(daemon.sock(), id);

    // 第二次 attach 应被拒绝
    let resp = Client::oneshot(daemon.sock(), &Request::Attach { session_id: id });
    assert!(
        matches!(resp, Response::AttachDeny { .. }),
        "expected AttachDeny, got: {resp:?}"
    );
}

#[test]
fn test_winsize_update() {
    let daemon = TestDaemon::start("winsize");
    let id = Client::create(daemon.sock());

    let resp = Client::oneshot(
        daemon.sock(),
        &Request::WinsizeUpdate {
            session_id: id,
            cols: 120,
            rows: 40,
        },
    );
    assert!(matches!(resp, Response::WinsizeUpdated { .. }));

    // 验证 list 中尺寸已更新
    match Client::oneshot(daemon.sock(), &Request::List) {
        Response::SessionList { sessions } => {
            let s = sessions.iter().find(|s| s.id == id).unwrap();
            assert_eq!(s.cols, 120);
            assert_eq!(s.rows, 40);
        }
        other => panic!("expected SessionList, got: {other:?}"),
    }
}

// ===========================================================================
// Detach winsize tests (previously part of baton-pass, now standalone)
// ===========================================================================

/// Helper: create a session with specific cols/rows
fn create_with_size(sock: &PathBuf, cols: u16, rows: u16) -> uuid::Uuid {
    match Client::oneshot(
        sock,
        &Request::Create {
            shell: None,
            cols,
            rows,
            working_dir: None,
            terminal_id: None,
            envs: None,
        },
    ) {
        Response::Created { session_id } => session_id,
        other => panic!("expected Created, got: {other:?}"),
    }
}

/// Helper: get session info (cols, rows, state) from List response
fn session_info(sock: &PathBuf, id: uuid::Uuid) -> Option<(u16, u16, String)> {
    match Client::oneshot(sock, &Request::List) {
        Response::SessionList { sessions } => sessions
            .iter()
            .find(|s| s.id == id)
            .map(|s| (s.cols, s.rows, s.state.clone())),
        _ => None,
    }
}

/// Detach with cols=0 rows=0 preserves the original winsize.
///
/// WS clients (iPhone) send cols=0, rows=0 when detaching to signal
/// "don't change PTY dimensions". The daemon must keep the existing
/// winsize unchanged.
#[test]
fn test_detach_zero_size_preserves_winsize() {
    let daemon = TestDaemon::start("zero-ws");
    let id = create_with_size(daemon.sock(), 120, 40);

    // Attach so the session transitions to Attached state
    let session = AttachedSession::attach(daemon.sock(), id);

    // Verify the session is Attached with original dimensions
    let (cols, rows, state) = session_info(daemon.sock(), id).unwrap();
    assert_eq!(state, "Attached");
    assert_eq!(cols, 120);
    assert_eq!(rows, 40);

    // Detach with cols=0, rows=0 (simulates WS client behavior)
    let resp = Client::oneshot(
        daemon.sock(),
        &Request::Detach {
            session_id: id,
            cols: 0,
            rows: 0,
            grid_snapshot: None,
        },
    );
    assert!(
        matches!(resp, Response::Detached { .. }),
        "expected Detached, got: {resp:?}"
    );

    // Drop the attached session handle (close pty_fd and control stream)
    drop(session);
    thread::sleep(Duration::from_millis(300));

    // Verify winsize is still 120x40 (not 0x0)
    let (cols, rows, state) = session_info(daemon.sock(), id).unwrap();
    assert_eq!(cols, 120, "cols must be preserved when detach sends 0");
    assert_eq!(rows, 40, "rows must be preserved when detach sends 0");
    assert_eq!(state, "Active");
}

/// Detach with non-zero cols/rows updates winsize.
#[test]
fn test_detach_nonzero_size_updates_winsize() {
    let daemon = TestDaemon::start("nonzero");
    let id = create_with_size(daemon.sock(), 80, 24);

    let session = AttachedSession::attach(daemon.sock(), id);

    // Detach with new dimensions
    let resp = Client::oneshot(
        daemon.sock(),
        &Request::Detach {
            session_id: id,
            cols: 132,
            rows: 50,
            grid_snapshot: None,
        },
    );
    assert!(matches!(resp, Response::Detached { .. }));

    drop(session);
    thread::sleep(Duration::from_millis(300));

    let (cols, rows, _) = session_info(daemon.sock(), id).unwrap();
    assert_eq!(cols, 132, "cols must update to new value");
    assert_eq!(rows, 50, "rows must update to new value");
}

/// WinsizeUpdate with same size on non-attached session returns
/// WinsizeUpdated (and triggers explicit SIGWINCH under the hood).
#[test]
fn test_winsize_update_same_size_returns_updated() {
    let daemon = TestDaemon::start("ws-samesize");
    let id = create_with_size(daemon.sock(), 100, 30);

    let resp = Client::oneshot(
        daemon.sock(),
        &Request::WinsizeUpdate {
            session_id: id,
            cols: 100,
            rows: 30,
        },
    );
    assert!(
        matches!(resp, Response::WinsizeUpdated { .. }),
        "expected WinsizeUpdated for same-size update, got: {resp:?}"
    );

    let (cols, rows, _) = session_info(daemon.sock(), id).unwrap();
    assert_eq!(cols, 100);
    assert_eq!(rows, 30);
}

/// WinsizeUpdate on an Attached session records the new dimensions
/// but does NOT call ioctl (ETerm does it directly on its dup_fd).
#[test]
fn test_winsize_update_while_attached_records_only() {
    let daemon = TestDaemon::start("ws-attached");
    let id = create_with_size(daemon.sock(), 80, 24);
    let _session = AttachedSession::attach(daemon.sock(), id);

    let resp = Client::oneshot(
        daemon.sock(),
        &Request::WinsizeUpdate {
            session_id: id,
            cols: 160,
            rows: 48,
        },
    );
    assert!(matches!(resp, Response::WinsizeUpdated { .. }));

    let (cols, rows, state) = session_info(daemon.sock(), id).unwrap();
    assert_eq!(state, "Attached");
    assert_eq!(cols, 160);
    assert_eq!(rows, 48);
}

/// Detach with zero size on a never-attached session preserves winsize.
#[test]
fn test_detach_never_attached_session() {
    let daemon = TestDaemon::start("noattach");
    let id = create_with_size(daemon.sock(), 100, 50);

    assert_eq!(
        Client::session_state(daemon.sock(), id),
        Some("Active".into())
    );

    let resp = Client::oneshot(
        daemon.sock(),
        &Request::Detach {
            session_id: id,
            cols: 0,
            rows: 0,
            grid_snapshot: None,
        },
    );
    assert!(
        matches!(resp, Response::Detached { .. }),
        "expected Detached for never-attached session, got: {resp:?}"
    );

    let (cols, rows, _) = session_info(daemon.sock(), id).unwrap();
    assert_eq!(cols, 100, "cols preserved on never-attached detach");
    assert_eq!(rows, 50, "rows preserved on never-attached detach");
}
