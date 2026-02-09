//! Session 数据结构与生命周期管理

use crate::ring_buffer::DEFAULT_RING_SIZE;
use crate::shared_ring::SharedRingBuffer;
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
    pub fn detach(&mut self) {
        self.state = SessionState::Active;
        self.owner_fd = None;
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
            unsafe { libc::kill(self.child_pid, libc::SIGHUP); }
        }
        unsafe { libc::close(self.master_fd); }
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
