//! 控制面协议消息定义
//!
//! 客户端（CLI / ETerm）通过 Unix socket 发送请求，daemon 返回响应。
//! 消息格式：4 字节 length (big-endian) + JSON payload

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// 协议版本
pub const PROTOCOL_VERSION: u16 = 1;

/// 控制面请求
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    /// 创建新 session（daemon 创建 PTY + spawn shell）
    Create {
        shell: Option<String>,
        cols: u16,
        rows: u16,
        working_dir: Option<String>,
        /// ETerm terminal_id，reattach 时用于精确映射
        #[serde(default)]
        terminal_id: Option<u32>,
        /// 额外的环境变量（如 ZDOTDIR, ETERM_SHELL_DIR）
        #[serde(default)]
        envs: Option<HashMap<String, String>>,
    },

    /// Attach 到已有 session（触发 fd passing）
    Attach { session_id: Uuid },

    /// 主动 Detach
    Detach {
        session_id: Uuid,
        cols: u16,
        rows: u16,
        /// base64-encoded GridSnapshot bytes (captured by ETerm before detach)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grid_snapshot: Option<String>,
    },

    /// 列出所有 session
    List,

    /// 杀掉 session
    Kill { session_id: Uuid },

    /// 更新 winsize（attached 时 ETerm 窗口 resize）
    WinsizeUpdate {
        session_id: Uuid,
        cols: u16,
        rows: u16,
    },

    /// Ping（健康检查）
    Ping,

    /// 请求 daemon 优雅退出
    Shutdown,
}

/// 控制面响应
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    /// Session 创建成功
    Created { session_id: Uuid },

    /// Attach 准备就绪，接下来走 fd passing + shm
    AttachReady {
        session_id: Uuid,
        cols: u16,
        rows: u16,
        child_pid: i32,
        /// 共享内存名称，客户端用于 open shm 读写终端输出
        shm_name: String,
        /// base64-encoded GridSnapshot bytes (captured on previous detach)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grid_snapshot: Option<String>,
    },

    /// Attach 被拒绝（已被其他客户端 attach）
    AttachDeny { session_id: Uuid, reason: String },

    /// Detach 确认
    Detached { session_id: Uuid },

    /// Session 列表
    SessionList { sessions: Vec<SessionInfo> },

    /// Session 已被 kill
    Killed { session_id: Uuid },

    /// Winsize 更新确认
    WinsizeUpdated { session_id: Uuid },

    /// Pong
    Pong { version: u16, session_count: usize },

    /// Daemon 正在关闭
    ShuttingDown,

    /// 错误
    Error { message: String },
}

/// Session 信息（用于 List 响应）
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: Uuid,
    pub state: String,
    pub child_pid: i32,
    pub cols: u16,
    pub rows: u16,
    pub ptsname: String,
    pub child_alive: bool,
    pub ring_buffer_bytes: u64,
    pub created_secs_ago: u64,
    pub last_active_secs_ago: u64,
    /// ETerm terminal_id（reattach 映射用）
    #[serde(default)]
    pub terminal_id: Option<u32>,
}

/// 编码消息：4 字节 length + JSON
pub fn encode_message<T: Serialize>(msg: &T) -> Vec<u8> {
    let json = serde_json::to_vec(msg).expect("serialize failed");
    let len = json.len() as u32;
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&json);
    buf
}

/// 从 buffer 中尝试解码一个完整消息
///
/// 返回 (解析结果, 消耗的字节数)
/// 如果数据不够返回 None
pub fn try_decode_message<T: for<'de> Deserialize<'de>>(
    buf: &[u8],
) -> Option<(Result<T, String>, usize)> {
    if buf.len() < 4 {
        return None;
    }

    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

    if buf.len() < 4 + len {
        return None;
    }

    let payload = &buf[4..4 + len];
    let result = serde_json::from_slice(payload).map_err(|e| e.to_string());
    Some((result, 4 + len))
}
