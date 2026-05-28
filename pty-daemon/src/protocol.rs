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

    /// 写入输入到 session 的 master_fd（takeover 期间 ETerm 输入转发）
    Input {
        session_id: Uuid,
        /// base64-encoded input bytes
        data: String,
    },
}

/// Server-initiated push messages (daemon → ETerm)
///
/// These are sent by the daemon to an attached ETerm client over the existing
/// control Unix socket. They are NOT responses to requests — they are
/// unsolicited push notifications that require the client to act.
///
/// Wire format: identical to Request/Response (4-byte BE length + JSON).
/// Discriminated by the `"type"` tag field (e.g. `"ForceDetach"`).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Push {
    /// Daemon requests ETerm to release a session (iPhone takeover).
    ///
    /// ETerm must:
    /// 1. Capture its grid snapshot
    /// 2. Close its dup(master_fd)
    /// 3. Send DetachAck back on the same control socket
    ForceDetach { session_id: Uuid },

    /// Daemon offers a session back to ETerm (iPhone released).
    ///
    /// Contains the daemon's grid snapshot captured while it was reading.
    /// ETerm must re-attach (gets new dup_fd) and apply the snapshot.
    ResumeAttach {
        session_id: Uuid,
        cols: u16,
        rows: u16,
        child_pid: i32,
        shm_name: String,
        /// base64-encoded GridSnapshot bytes (daemon's view while it was reading)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grid_snapshot: Option<String>,
    },
}

/// Client acknowledgment of a Push message (ETerm → daemon)
///
/// Sent over the same control socket in response to a Push.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PushAck {
    /// ETerm has released the session and provides its grid snapshot.
    DetachAck {
        session_id: Uuid,
        /// base64-encoded GridSnapshot bytes captured by ETerm before closing dup_fd
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grid_snapshot: Option<String>,
    },

    /// ETerm has re-attached and applied the snapshot.
    ResumeAck { session_id: Uuid },
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

    /// Input 已投递
    InputAck { session_id: Uuid },

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

#[cfg(test)]
mod tests {
    use super::*;

    // B16: Push::ForceDetach serialize/deserialize roundtrip
    #[test]
    fn push_force_detach_roundtrip() {
        let session_id = Uuid::new_v4();
        let msg = Push::ForceDetach { session_id };

        let json = serde_json::to_string(&msg).unwrap();

        // The JSON tag must be "ForceDetach"
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(raw["type"], "ForceDetach");
        assert_eq!(raw["session_id"], session_id.to_string());

        // Deserialize back and verify session_id is preserved
        let decoded: Push = serde_json::from_str(&json).unwrap();
        match decoded {
            Push::ForceDetach { session_id: sid } => {
                assert_eq!(sid, session_id);
            }
            other => panic!("expected ForceDetach, got: {other:?}"),
        }
    }

    // B17: Push::ResumeAttach with and without grid_snapshot
    #[test]
    fn push_resume_attach_with_snapshot() {
        let session_id = Uuid::new_v4();
        let snapshot = Some("base64data==".to_string());
        let msg = Push::ResumeAttach {
            session_id,
            cols: 120,
            rows: 40,
            child_pid: 12345,
            shm_name: "/ptyd-test".to_string(),
            grid_snapshot: snapshot.clone(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(raw["type"], "ResumeAttach");
        assert_eq!(raw["grid_snapshot"], "base64data==");

        let decoded: Push = serde_json::from_str(&json).unwrap();
        match decoded {
            Push::ResumeAttach {
                session_id: sid,
                cols,
                rows,
                child_pid,
                shm_name,
                grid_snapshot,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(cols, 120);
                assert_eq!(rows, 40);
                assert_eq!(child_pid, 12345);
                assert_eq!(shm_name, "/ptyd-test");
                assert_eq!(grid_snapshot, snapshot);
            }
            other => panic!("expected ResumeAttach, got: {other:?}"),
        }
    }

    #[test]
    fn push_resume_attach_without_snapshot() {
        let session_id = Uuid::new_v4();
        let msg = Push::ResumeAttach {
            session_id,
            cols: 80,
            rows: 24,
            child_pid: 999,
            shm_name: "/ptyd-nosnapshot".to_string(),
            grid_snapshot: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();

        // skip_serializing_if = "Option::is_none" must omit the field entirely
        assert!(
            raw.get("grid_snapshot").is_none(),
            "grid_snapshot should be absent when None, got: {json}"
        );

        let decoded: Push = serde_json::from_str(&json).unwrap();
        match decoded {
            Push::ResumeAttach { grid_snapshot, .. } => {
                assert_eq!(grid_snapshot, None);
            }
            other => panic!("expected ResumeAttach, got: {other:?}"),
        }
    }

    // B18: PushAck::DetachAck with and without grid_snapshot
    #[test]
    fn push_ack_detach_ack_with_snapshot() {
        let session_id = Uuid::new_v4();
        let msg = PushAck::DetachAck {
            session_id,
            grid_snapshot: Some("snapshot_data".to_string()),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(raw["type"], "DetachAck");
        assert_eq!(raw["grid_snapshot"], "snapshot_data");

        let decoded: PushAck = serde_json::from_str(&json).unwrap();
        match decoded {
            PushAck::DetachAck {
                session_id: sid,
                grid_snapshot,
            } => {
                assert_eq!(sid, session_id);
                assert_eq!(grid_snapshot, Some("snapshot_data".to_string()));
            }
            other => panic!("expected DetachAck, got: {other:?}"),
        }
    }

    #[test]
    fn push_ack_detach_ack_without_snapshot() {
        let session_id = Uuid::new_v4();
        let msg = PushAck::DetachAck {
            session_id,
            grid_snapshot: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();

        // skip_serializing_if must omit the field
        assert!(
            raw.get("grid_snapshot").is_none(),
            "grid_snapshot should be absent when None, got: {json}"
        );

        let decoded: PushAck = serde_json::from_str(&json).unwrap();
        match decoded {
            PushAck::DetachAck { grid_snapshot, .. } => {
                assert_eq!(grid_snapshot, None);
            }
            other => panic!("expected DetachAck, got: {other:?}"),
        }
    }

    // B19: 4-byte BE length prefix encode_message → try_decode_message roundtrip
    #[test]
    fn length_prefix_roundtrip_push() {
        let session_id = Uuid::new_v4();
        let original = Push::ForceDetach { session_id };

        let encoded = encode_message(&original);

        // First 4 bytes are BE length
        let payload_len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
        assert_eq!(payload_len as usize, encoded.len() - 4);

        // Full decode
        let (result, consumed) = try_decode_message::<Push>(&encoded).expect("buffer should be complete");
        assert_eq!(consumed, encoded.len());
        let decoded = result.expect("deserialization should succeed");
        match decoded {
            Push::ForceDetach { session_id: sid } => assert_eq!(sid, session_id),
            other => panic!("expected ForceDetach, got: {other:?}"),
        }
    }

    #[test]
    fn length_prefix_roundtrip_push_ack() {
        let session_id = Uuid::new_v4();
        let original = PushAck::ResumeAck { session_id };

        let encoded = encode_message(&original);

        let payload_len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]);
        assert_eq!(payload_len as usize, encoded.len() - 4);

        let (result, consumed) =
            try_decode_message::<PushAck>(&encoded).expect("buffer should be complete");
        assert_eq!(consumed, encoded.len());
        let decoded = result.expect("deserialization should succeed");
        match decoded {
            PushAck::ResumeAck { session_id: sid } => assert_eq!(sid, session_id),
            other => panic!("expected ResumeAck, got: {other:?}"),
        }
    }

    #[test]
    fn try_decode_returns_none_on_incomplete_header() {
        // Less than 4 bytes: should return None (not enough data)
        assert!(try_decode_message::<Push>(&[0u8; 3]).is_none());
    }

    #[test]
    fn try_decode_returns_none_on_incomplete_payload() {
        // Header says 100 bytes but only 10 bytes of payload provided
        let mut buf = Vec::new();
        buf.extend_from_slice(&100u32.to_be_bytes());
        buf.extend_from_slice(&[0u8; 10]);
        assert!(try_decode_message::<Push>(&buf).is_none());
    }
}
