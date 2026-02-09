//! 固定大小 circular byte buffer
//!
//! Tier 3 模式下 daemon 只持 fd + ring buffer，零 CPU。
//! reattach 时 dump 出来前插 terminal reset 序列做 replay。

/// 默认 1MB ring buffer
pub const DEFAULT_RING_SIZE: usize = 1024 * 1024;

pub struct RingBuffer {
    buf: Box<[u8]>,
    /// 下一个写入位置
    write_pos: usize,
    /// 已写入的总字节数（用于判断是否回绕过）
    total_written: u64,
    capacity: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0u8; capacity].into_boxed_slice(),
            write_pos: 0,
            total_written: 0,
            capacity,
        }
    }

    /// 写入数据，自动回绕覆盖旧数据
    pub fn write(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        let len = data.len();

        if len >= self.capacity {
            // 数据比 buffer 大，只保留末尾 capacity 字节
            let start = len - self.capacity;
            self.buf.copy_from_slice(&data[start..]);
            self.write_pos = 0;
            self.total_written += len as u64;
            return;
        }

        let first_chunk = self.capacity - self.write_pos;
        if len <= first_chunk {
            // 不需要回绕
            self.buf[self.write_pos..self.write_pos + len].copy_from_slice(data);
        } else {
            // 需要回绕
            self.buf[self.write_pos..self.write_pos + first_chunk]
                .copy_from_slice(&data[..first_chunk]);
            let remaining = len - first_chunk;
            self.buf[..remaining].copy_from_slice(&data[first_chunk..]);
        }

        self.write_pos = (self.write_pos + len) % self.capacity;
        self.total_written += len as u64;
    }

    /// 当前有效数据长度
    pub fn len(&self) -> usize {
        if self.total_written >= self.capacity as u64 {
            self.capacity
        } else {
            self.total_written as usize
        }
    }

    pub fn is_empty(&self) -> bool {
        self.total_written == 0
    }

    /// Dump 有效数据（按时间顺序），用于 replay
    ///
    /// 返回的数据可以直接喂给 terminal emulator 恢复状态
    pub fn dump(&self) -> Vec<u8> {
        let valid_len = self.len();
        if valid_len == 0 {
            return Vec::new();
        }

        let mut result = Vec::with_capacity(valid_len);

        if self.total_written >= self.capacity as u64 {
            // 有回绕：从 write_pos 开始读到末尾，再从头读到 write_pos
            result.extend_from_slice(&self.buf[self.write_pos..]);
            result.extend_from_slice(&self.buf[..self.write_pos]);
        } else {
            // 无回绕：从头读到 write_pos
            result.extend_from_slice(&self.buf[..self.write_pos]);
        }

        result
    }

    /// Dump 带 terminal reset 前缀，用于 Tier 3 → Tier 2 恢复
    ///
    /// ESC[!p (DECSTR) 重置终端状态，再 replay ring buffer 内容
    pub fn dump_with_reset(&self) -> Vec<u8> {
        let data = self.dump();
        if data.is_empty() {
            return data;
        }

        // ESC[!p = Soft Terminal Reset (DECSTR)
        let mut result = Vec::with_capacity(data.len() + 4);
        result.extend_from_slice(b"\x1b[!p");
        result.extend_from_slice(&data);
        result
    }

    pub fn clear(&mut self) {
        self.write_pos = 0;
        self.total_written = 0;
    }

    pub fn total_written(&self) -> u64 {
        self.total_written
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_write_read() {
        let mut rb = RingBuffer::new(16);
        rb.write(b"hello");
        assert_eq!(rb.len(), 5);
        assert_eq!(rb.dump(), b"hello");
    }

    #[test]
    fn test_wraparound() {
        let mut rb = RingBuffer::new(8);
        rb.write(b"12345678"); // 填满
        assert_eq!(rb.len(), 8);
        assert_eq!(rb.dump(), b"12345678");

        rb.write(b"ab"); // 回绕覆盖前 2 字节
        assert_eq!(rb.len(), 8);
        assert_eq!(rb.dump(), b"345678ab");
    }

    #[test]
    fn test_overflow_write() {
        let mut rb = RingBuffer::new(4);
        rb.write(b"abcdefgh"); // 写入 > capacity
        assert_eq!(rb.len(), 4);
        assert_eq!(rb.dump(), b"efgh");
    }

    #[test]
    fn test_dump_with_reset() {
        let mut rb = RingBuffer::new(16);
        rb.write(b"test");
        let data = rb.dump_with_reset();
        assert_eq!(&data[..4], b"\x1b[!p");
        assert_eq!(&data[4..], b"test");
    }

    #[test]
    fn test_empty() {
        let rb = RingBuffer::new(16);
        assert!(rb.is_empty());
        assert_eq!(rb.len(), 0);
        assert!(rb.dump().is_empty());
        assert!(rb.dump_with_reset().is_empty());
    }

    #[test]
    fn test_clear() {
        let mut rb = RingBuffer::new(16);
        rb.write(b"data");
        rb.clear();
        assert!(rb.is_empty());
        assert_eq!(rb.dump(), b"");
    }
}
