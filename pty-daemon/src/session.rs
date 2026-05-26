//! Session 数据结构与生命周期管理

use crate::ring_buffer::DEFAULT_RING_SIZE;
use crate::shared_ring::SharedRingBuffer;
use crate::terminal_state::TerminalState;
use std::collections::HashMap;
use std::os::fd::RawFd;
use std::time::Instant;
use uuid::Uuid;

/// Session 运行状态（对应设计文档 Tier）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Tier 1: ETerm attached，ETerm 直持 dup(master_fd)，daemon 休眠
    Attached,
    /// Tier 2: 有活跃输出，daemon 跑读循环（Phase 1 不实现 Machine）
    Active,
    /// Tier 3: 空闲，只持 fd + ring buffer
    Idle,
}

/// 窗口大小
#[derive(Debug, Clone, Copy)]
pub struct WinSize {
    pub cols: u16,
    pub rows: u16,
}

/// 单个终端 session
pub struct Session {
    pub id: Uuid,
    pub master_fd: RawFd,
    pub child_pid: libc::pid_t,
    pub ptsname: String,
    pub state: SessionState,
    pub winsize: WinSize,
    /// 共享内存 ring buffer（替代进程内 RingBuffer）
    pub shared_ring: SharedRingBuffer,
    /// 共享内存名称
    pub shm_name: String,

    /// 创建时间
    pub created_at: Instant,
    /// 最后活跃时间（最后一次有输出）
    pub last_active: Instant,
    /// 最后 attach 时间
    pub last_attached: Option<Instant>,
    /// 当前 attach 的客户端 owner（控制 socket fd）
    pub owner_fd: Option<RawFd>,
    /// ETerm terminal_id（reattach 时精确映射）
    pub terminal_id: Option<u32>,
    /// Grid snapshot bytes (base64-encoded, from ETerm on detach)
    pub grid_snapshot: Option<String>,
    /// Daemon-side ANSI parser state for crash-recovery snapshots
    pub terminal_state: Option<TerminalState>,
    /// Number of WebSocket clients currently attached to this session
    pub ws_client_count: u32,
    /// Previous owner fd (ETerm's control socket) — saved during baton-pass takeover
    /// so that baton-release can push ResumeAttach back to ETerm.
    pub previous_owner_fd: Option<RawFd>,
}

impl Session {
    pub fn new(
        master_fd: RawFd,
        child_pid: libc::pid_t,
        ptsname: String,
        winsize: WinSize,
        terminal_id: Option<u32>,
    ) -> Self {
        let now = Instant::now();
        let id = Uuid::new_v4();
        let shm_name = crate::shared_ring::shm_name_for_session(&id);
        let shared_ring = SharedRingBuffer::create(&shm_name, DEFAULT_RING_SIZE)
            .unwrap_or_else(|e| panic!("failed to create shared ring buffer {shm_name}: {e}"));
        Self {
            id,
            master_fd,
            child_pid,
            ptsname,
            state: SessionState::Active,
            winsize,
            shared_ring,
            shm_name,
            created_at: now,
            last_active: now,
            last_attached: None,
            owner_fd: None,
            terminal_id,
            grid_snapshot: None,
            terminal_state: Some(TerminalState::new(winsize.cols, winsize.rows)),
            ws_client_count: 0,
            previous_owner_fd: None,
        }
    }

    /// 子进程是否还活着
    pub fn is_child_alive(&self) -> bool {
        // kill(pid, 0) 不发信号，只检查进程是否存在
        unsafe { libc::kill(self.child_pid, 0) == 0 }
    }

    /// 转入 attached 状态（ETerm 直持 dup(master_fd)，daemon 休眠）
    pub fn attach(&mut self, owner_fd: RawFd) {
        self.state = SessionState::Attached;
        self.owner_fd = Some(owner_fd);
        self.last_attached = Some(Instant::now());
    }

    /// 转入 detached 状态（主动或崩溃），daemon 重新接管 master_fd
    pub fn detach(&mut self, snapshot: Option<String>) {
        self.state = SessionState::Active;
        self.owner_fd = None;
        if snapshot.is_some() {
            self.grid_snapshot = snapshot;
        }
    }

    /// Baton-pass detach: save previous owner before clearing, for later baton-release.
    ///
    /// Call this instead of `detach()` when the detach is caused by a WS takeover
    /// (iPhone takes over from ETerm). The previous_owner_fd is used to push
    /// ResumeAttach back to ETerm when the iPhone releases.
    pub fn baton_detach(&mut self, snapshot: Option<String>) {
        if self.owner_fd.is_some() {
            self.previous_owner_fd = self.owner_fd;
        }
        self.detach(snapshot);
    }

    /// 降级到 idle（Tier 2 → Tier 3）
    pub fn degrade_to_idle(&mut self) {
        self.state = SessionState::Idle;
    }

    /// 升级到 active（Tier 3 → Tier 2）
    pub fn promote_to_active(&mut self) {
        self.state = SessionState::Active;
        self.last_active = Instant::now();
    }

    /// 清理：关 fd，杀进程，删除共享内存
    pub fn cleanup(&mut self) {
        if self.is_child_alive() {
            unsafe {
                libc::kill(self.child_pid, libc::SIGHUP);
            }
        }
        unsafe {
            libc::close(self.master_fd);
        }
        self.master_fd = -1;
        // 删除共享内存对象
        if let Err(e) = self.shared_ring.unlink() {
            eprintln!("[session] failed to unlink shm {}: {e}", self.shm_name);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.master_fd >= 0 {
            self.cleanup();
        }
    }
}

/// Session 管理器
pub struct SessionManager {
    sessions: HashMap<Uuid, Session>,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn add(&mut self, session: Session) -> Uuid {
        let id = session.id;
        self.sessions.insert(id, session);
        id
    }

    pub fn get(&self, id: &Uuid) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: &Uuid) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    pub fn remove(&mut self, id: &Uuid) -> Option<Session> {
        self.sessions.remove(id)
    }

    pub fn list(&self) -> Vec<&Session> {
        let mut v: Vec<&Session> = self.sessions.values().collect();
        v.sort_by_key(|s| s.created_at);
        v
    }

    /// Mutable access to all sessions (for bulk updates like clearing previous_owner_fd)
    pub fn list_mut(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.values_mut()
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    /// 通过 master_fd 查找 session
    pub fn find_by_master_fd(&self, fd: RawFd) -> Option<&Session> {
        self.sessions.values().find(|s| s.master_fd == fd)
    }

    /// 通过 child_pid 查找 session
    pub fn find_by_child_pid(&self, pid: libc::pid_t) -> Option<&Session> {
        self.sessions.values().find(|s| s.child_pid == pid)
    }

    /// 清理所有子进程已退出的 session，返回 (id, master_fd) 列表
    pub fn reap_dead(&mut self) -> Vec<(Uuid, RawFd)> {
        let dead: Vec<(Uuid, RawFd)> = self
            .sessions
            .iter()
            .filter(|(_, s)| !s.is_child_alive())
            .map(|(id, s)| (*id, s.master_fd))
            .collect();

        for (id, _) in &dead {
            if let Some(session) = self.sessions.get(id) {
                // waitpid 回收 zombie
                let mut status: libc::c_int = 0;
                unsafe {
                    libc::waitpid(session.child_pid, &mut status, libc::WNOHANG);
                }
            }
            self.sessions.remove(id);
        }

        dead
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_ring::SharedRingBuffer;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Monotonic counter to generate unique shm names across tests.
    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Build a minimal Session for unit testing.
    ///
    /// Uses master_fd = -1 so Drop::cleanup is a no-op (guarded by `if self.master_fd >= 0`).
    /// Creates real shm so SharedRingBuffer is valid; caller must call `unlink_shm()` on the
    /// returned helper or let the helper's Drop handle it.
    fn test_session() -> (Session, TestShmGuard) {
        let seq = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let shm_name = format!("/ptyd-sess-test-{}-{}", std::process::id(), seq);
        let shared_ring = SharedRingBuffer::create(&shm_name, 4096)
            .expect("create test shm");

        let session = Session {
            id: Uuid::new_v4(),
            master_fd: -1, // sentinel — prevents Drop from closing/killing anything
            child_pid: 0,
            ptsname: String::new(),
            state: SessionState::Active,
            winsize: WinSize { cols: 80, rows: 24 },
            shared_ring,
            shm_name: shm_name.clone(),
            created_at: Instant::now(),
            last_active: Instant::now(),
            last_attached: None,
            owner_fd: None,
            terminal_id: None,
            grid_snapshot: None,
            terminal_state: None,
            ws_client_count: 0,
            previous_owner_fd: None,
        };

        (session, TestShmGuard(shm_name))
    }

    /// RAII guard that unlinks the test shm on drop.
    struct TestShmGuard(String);

    impl Drop for TestShmGuard {
        fn drop(&mut self) {
            unsafe {
                let c_name = std::ffi::CString::new(self.0.as_str()).unwrap();
                libc::shm_unlink(c_name.as_ptr());
            }
        }
    }

    // B3: baton_detach preserves previous_owner_fd
    #[test]
    fn baton_detach_preserves_previous_owner_fd() {
        let (mut session, _guard) = test_session();
        let fake_owner_fd: RawFd = 42;

        // Simulate an attached state with an owner
        session.attach(fake_owner_fd);
        assert_eq!(session.state, SessionState::Attached);
        assert_eq!(session.owner_fd, Some(fake_owner_fd));
        assert_eq!(session.previous_owner_fd, None);

        // Baton-detach (iPhone takeover) should save the owner before clearing
        session.baton_detach(Some("snapshot_from_eterm".to_string()));

        assert_eq!(session.owner_fd, None, "owner_fd must be cleared");
        assert_eq!(
            session.previous_owner_fd,
            Some(fake_owner_fd),
            "previous_owner_fd must preserve the original owner"
        );
        assert_eq!(session.state, SessionState::Active);
        assert_eq!(
            session.grid_snapshot,
            Some("snapshot_from_eterm".to_string())
        );
    }

    // B3 extended: baton_detach with None snapshot preserves existing snapshot
    #[test]
    fn baton_detach_with_none_snapshot_keeps_existing() {
        let (mut session, _guard) = test_session();

        session.grid_snapshot = Some("old_snapshot".to_string());
        session.attach(10);
        session.baton_detach(None);

        // detach() only updates grid_snapshot when Some is passed
        assert_eq!(
            session.grid_snapshot,
            Some("old_snapshot".to_string()),
            "existing snapshot must be preserved when baton_detach passes None"
        );
        assert_eq!(session.previous_owner_fd, Some(10));
    }

    // B5: Normal detach does not set previous_owner_fd
    #[test]
    fn normal_detach_leaves_previous_owner_fd_none() {
        let (mut session, _guard) = test_session();
        let fake_owner_fd: RawFd = 99;

        session.attach(fake_owner_fd);
        assert_eq!(session.owner_fd, Some(fake_owner_fd));
        assert_eq!(session.previous_owner_fd, None);

        // Normal detach (user-initiated or crash recovery)
        session.detach(Some("normal_snapshot".to_string()));

        assert_eq!(session.owner_fd, None, "owner_fd must be cleared");
        assert_eq!(
            session.previous_owner_fd, None,
            "normal detach must not touch previous_owner_fd"
        );
        assert_eq!(session.state, SessionState::Active);
        assert_eq!(
            session.grid_snapshot,
            Some("normal_snapshot".to_string())
        );
    }

    // B5 extended: repeated normal detach never populates previous_owner_fd
    #[test]
    fn repeated_normal_detach_keeps_previous_owner_none() {
        let (mut session, _guard) = test_session();

        // Cycle: attach → detach → attach → detach
        session.attach(50);
        session.detach(None);
        assert_eq!(session.previous_owner_fd, None);

        session.attach(60);
        session.detach(Some("snap2".to_string()));
        assert_eq!(session.previous_owner_fd, None);
        assert_eq!(session.grid_snapshot, Some("snap2".to_string()));
    }

    // Baton-detach when owner_fd is already None should not overwrite previous_owner_fd
    #[test]
    fn baton_detach_without_owner_does_not_clobber_previous() {
        let (mut session, _guard) = test_session();

        // Set a previous_owner_fd from an earlier baton cycle
        session.previous_owner_fd = Some(77);

        // Session is not currently attached (owner_fd is None)
        assert_eq!(session.owner_fd, None);

        // baton_detach should skip the save (guarded by `if self.owner_fd.is_some()`)
        session.baton_detach(None);

        assert_eq!(
            session.previous_owner_fd,
            Some(77),
            "must not clobber previous_owner_fd when owner_fd is None"
        );
    }
}
