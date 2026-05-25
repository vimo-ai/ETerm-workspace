//! 基于 POSIX 共享内存的环形缓冲区
//!
//! 用于跨进程共享 PTY 输出数据。采用 128-byte cache line 对齐（Apple Silicon），
//! 单 writer（daemon session）多 reader（MCP clients）模型。
//!
//! ## 内存布局
//! ```text
//! Offset 0-127:    Header  { magic: u32, version: u32, capacity: u64, reserved }
//! Offset 128-255:  Slot    { write_pos: AtomicU64, padding }
//! Offset 256-383:  Slot    { total_written: AtomicU64, padding }
//! Offset 384+:     data[0..capacity]
//! ```

use std::io;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicU64, Ordering};

/// Header 大小：3 个 128-byte cache lines
pub const HEADER_SIZE: usize = 384;

/// 共享内存魔数 "PTYD"
pub const SHM_MAGIC: u32 = 0x50545944;

/// 共享内存版本
pub const SHM_VERSION: u32 = 1;

/// 默认 ring buffer 容量（与 ring_buffer.rs 保持一致）
pub const DEFAULT_RING_SIZE: usize = 1024 * 1024;

/// 共享内存环形缓冲区
///
/// 通过 POSIX shm_open/mmap 实现跨进程共享。
/// 采用 lock-free 设计：单 writer 通过 atomic 更新 write_pos，
/// 多 reader 通过 Acquire ordering 读取一致性快照。
pub struct SharedRingBuffer {
    shm_name: String,
    shm_fd: RawFd,
    mmap_ptr: *mut u8,
    mmap_size: usize,
    capacity: usize,
    data_ptr: *mut u8,
    is_owner: bool,
}

unsafe impl Send for SharedRingBuffer {}

impl SharedRingBuffer {
    /// 创建新的共享内存环形缓冲区
    ///
    /// # Arguments
    /// * `shm_name` - 共享内存名称（如 "/ptyd-abc12345"）
    /// * `capacity` - 数据区容量（字节）
    ///
    /// # Errors
    /// 如果共享内存已存在、权限不足或系统调用失败则返回错误
    pub fn create(shm_name: &str, capacity: usize) -> io::Result<Self> {
        // 1. 先尝试 unlink，清理可能的孤儿对象（忽略错误）
        unsafe {
            let c_name = std::ffi::CString::new(shm_name)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid shm name"))?;
            libc::shm_unlink(c_name.as_ptr());
        }

        // 2. 创建共享内存对象（O_CREAT | O_EXCL | O_RDWR）
        let shm_fd = unsafe {
            let c_name = std::ffi::CString::new(shm_name)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid shm name"))?;
            let fd = libc::shm_open(
                c_name.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                0o600,
            );
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            fd
        };

        // 3. 计算 mmap 大小（页对齐）
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
        let required_size = HEADER_SIZE + capacity;
        let mmap_size = required_size.div_ceil(page_size) * page_size;

        // 4. ftruncate 设置大小
        let truncate_result = unsafe { libc::ftruncate(shm_fd, mmap_size as i64) };
        if truncate_result != 0 {
            let err = io::Error::last_os_error();
            unsafe { libc::close(shm_fd) };
            return Err(err);
        }

        // 5. mmap 映射内存
        let mmap_ptr = unsafe {
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                mmap_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                shm_fd,
                0,
            );
            if ptr == libc::MAP_FAILED {
                let err = io::Error::last_os_error();
                libc::close(shm_fd);
                return Err(err);
            }
            ptr as *mut u8
        };

        // 6. 初始化 header
        unsafe {
            // magic + version
            std::ptr::write(mmap_ptr as *mut u32, SHM_MAGIC);
            std::ptr::write(mmap_ptr.add(4) as *mut u32, SHM_VERSION);
            // capacity
            std::ptr::write(mmap_ptr.add(8) as *mut u64, capacity as u64);
            // reserved 区域清零
            std::ptr::write_bytes(mmap_ptr.add(16), 0, 128 - 16);
        }

        // 7. 初始化 atomic 字段（write_pos 和 total_written）
        let data_ptr = unsafe { mmap_ptr.add(HEADER_SIZE) };
        let ring = SharedRingBuffer {
            shm_name: shm_name.to_string(),
            shm_fd,
            mmap_ptr,
            mmap_size,
            capacity,
            data_ptr,
            is_owner: true,
        };

        ring.write_pos_atomic().store(0, Ordering::Release);
        ring.total_written_atomic().store(0, Ordering::Release);

        Ok(ring)
    }

    /// 打开已存在的共享内存环形缓冲区
    ///
    /// # Arguments
    /// * `shm_name` - 共享内存名称
    ///
    /// # Errors
    /// 如果共享内存不存在、magic/version 校验失败或系统调用失败则返回错误
    pub fn open(shm_name: &str) -> io::Result<Self> {
        // 1. 打开共享内存对象（O_RDWR，不创建）
        let shm_fd = unsafe {
            let c_name = std::ffi::CString::new(shm_name)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid shm name"))?;
            let fd = libc::shm_open(c_name.as_ptr(), libc::O_RDWR, 0);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            fd
        };

        // 2. fstat 获取大小
        let mmap_size = unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            if libc::fstat(shm_fd, &mut stat) != 0 {
                let err = io::Error::last_os_error();
                libc::close(shm_fd);
                return Err(err);
            }
            stat.st_size as usize
        };

        if mmap_size < HEADER_SIZE {
            unsafe { libc::close(shm_fd) };
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Shared memory too small",
            ));
        }

        // 3. mmap 映射内存
        let mmap_ptr = unsafe {
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                mmap_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                shm_fd,
                0,
            );
            if ptr == libc::MAP_FAILED {
                let err = io::Error::last_os_error();
                libc::close(shm_fd);
                return Err(err);
            }
            ptr as *mut u8
        };

        // 4. 校验 magic 和 version
        let magic = unsafe { std::ptr::read(mmap_ptr as *const u32) };
        let version = unsafe { std::ptr::read(mmap_ptr.add(4) as *const u32) };

        if magic != SHM_MAGIC {
            unsafe {
                libc::munmap(mmap_ptr as *mut libc::c_void, mmap_size);
                libc::close(shm_fd);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid magic number",
            ));
        }

        if version != SHM_VERSION {
            unsafe {
                libc::munmap(mmap_ptr as *mut libc::c_void, mmap_size);
                libc::close(shm_fd);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported version",
            ));
        }

        // 5. 读取 capacity
        let capacity = unsafe { std::ptr::read(mmap_ptr.add(8) as *const u64) } as usize;

        if mmap_size < HEADER_SIZE + capacity {
            unsafe {
                libc::munmap(mmap_ptr as *mut libc::c_void, mmap_size);
                libc::close(shm_fd);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Capacity mismatch",
            ));
        }

        let data_ptr = unsafe { mmap_ptr.add(HEADER_SIZE) };

        Ok(SharedRingBuffer {
            shm_name: shm_name.to_string(),
            shm_fd,
            mmap_ptr,
            mmap_size,
            capacity,
            data_ptr,
            is_owner: false,
        })
    }

    /// 写入数据到环形缓冲区（单 writer 模型）
    ///
    /// 自动处理回绕和覆盖。如果 data.len() >= capacity，只保留末尾数据。
    pub fn write(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        let len = data.len();
        let write_pos_atom = self.write_pos_atomic();
        let total_atom = self.total_written_atomic();

        // 读取当前位置
        let mut write_pos = write_pos_atom.load(Ordering::Acquire);
        let total_written = total_atom.load(Ordering::Acquire);

        if len >= self.capacity {
            // 数据比 buffer 大，只保留末尾 capacity 字节
            let start = len - self.capacity;
            unsafe {
                std::ptr::copy_nonoverlapping(data[start..].as_ptr(), self.data_ptr, self.capacity);
            }
            write_pos = 0;
            write_pos_atom.store(write_pos, Ordering::Release);
            total_atom.store(total_written + len as u64, Ordering::Release);
            return;
        }

        let first_chunk = self.capacity - write_pos as usize;
        if len <= first_chunk {
            // 不需要回绕
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    self.data_ptr.add(write_pos as usize),
                    len,
                );
            }
        } else {
            // 需要回绕
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.as_ptr(),
                    self.data_ptr.add(write_pos as usize),
                    first_chunk,
                );
                let remaining = len - first_chunk;
                std::ptr::copy_nonoverlapping(
                    data[first_chunk..].as_ptr(),
                    self.data_ptr,
                    remaining,
                );
            }
        }

        write_pos = ((write_pos as usize + len) % self.capacity) as u64;
        write_pos_atom.store(write_pos, Ordering::Release);
        total_atom.store(total_written + len as u64, Ordering::Release);
    }

    /// Dump 当前有效数据（按时间顺序）
    ///
    /// 返回的数据可以直接用于 terminal replay
    pub fn dump(&self) -> Vec<u8> {
        // 校验 magic
        let magic = unsafe { std::ptr::read(self.mmap_ptr as *const u32) };
        if magic != SHM_MAGIC {
            return Vec::new();
        }

        let write_pos = self.write_pos_atomic().load(Ordering::Acquire);
        let total_written = self.total_written_atomic().load(Ordering::Acquire);

        // 边界检查
        if write_pos >= self.capacity as u64 {
            return Vec::new();
        }

        let valid_len = if total_written >= self.capacity as u64 {
            self.capacity
        } else {
            total_written as usize
        };

        if valid_len == 0 {
            return Vec::new();
        }

        let mut result = Vec::with_capacity(valid_len);

        if total_written >= self.capacity as u64 {
            // 有回绕：从 write_pos 开始读到末尾，再从头读到 write_pos
            let tail_len = self.capacity - write_pos as usize;
            unsafe {
                let tail =
                    std::slice::from_raw_parts(self.data_ptr.add(write_pos as usize), tail_len);
                result.extend_from_slice(tail);
                let head = std::slice::from_raw_parts(self.data_ptr, write_pos as usize);
                result.extend_from_slice(head);
            }
        } else {
            // 无回绕：从头读到 write_pos
            unsafe {
                let data = std::slice::from_raw_parts(self.data_ptr, write_pos as usize);
                result.extend_from_slice(data);
            }
        }

        result
    }

    /// Dump 带 terminal reset 前缀
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

    /// 当前有效数据长度
    pub fn len(&self) -> usize {
        let total_written = self.total_written_atomic().load(Ordering::Acquire);
        if total_written >= self.capacity as u64 {
            self.capacity
        } else {
            total_written as usize
        }
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.total_written_atomic().load(Ordering::Acquire) == 0
    }

    /// 已写入的总字节数
    pub fn total_written(&self) -> u64 {
        self.total_written_atomic().load(Ordering::Acquire)
    }

    /// 清空缓冲区
    pub fn clear(&self) {
        self.write_pos_atomic().store(0, Ordering::Release);
        self.total_written_atomic().store(0, Ordering::Release);
    }

    /// 获取容量
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 获取共享内存名称
    pub fn shm_name(&self) -> &str {
        &self.shm_name
    }

    /// 删除共享内存对象（仅 owner 应该调用）
    ///
    /// 调用后其他进程仍可访问已打开的映射，但无法再通过 shm_name 打开。
    pub fn unlink(&self) -> io::Result<()> {
        unsafe {
            let c_name = std::ffi::CString::new(self.shm_name.as_str())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid shm name"))?;
            if libc::shm_unlink(c_name.as_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// 获取 write_pos atomic 引用（offset 128）
    fn write_pos_atomic(&self) -> &AtomicU64 {
        unsafe { &*(self.mmap_ptr.add(128) as *const AtomicU64) }
    }

    /// 获取 total_written atomic 引用（offset 256）
    fn total_written_atomic(&self) -> &AtomicU64 {
        unsafe { &*(self.mmap_ptr.add(256) as *const AtomicU64) }
    }
}

impl Drop for SharedRingBuffer {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.mmap_ptr as *mut libc::c_void, self.mmap_size);
            libc::close(self.shm_fd);
        }
        // 注意：不自动 unlink，必须显式调用 unlink()
    }
}

/// 为 session ID 生成共享内存名称
///
/// 格式："/ptyd-{前8位hex}"
pub fn shm_name_for_session(id: &uuid::Uuid) -> String {
    format!("/ptyd-{}", &id.to_string()[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_shm_name(prefix: &str) -> String {
        format!("/ptyd-test-{}-{}", prefix, std::process::id())
    }

    #[test]
    fn test_basic_write_read() {
        let shm_name = test_shm_name("basic");
        let ring = SharedRingBuffer::create(&shm_name, 16).unwrap();

        ring.write(b"hello");
        assert_eq!(ring.len(), 5);
        assert_eq!(ring.dump(), b"hello");

        ring.unlink().unwrap();
    }

    #[test]
    fn test_wraparound() {
        let shm_name = test_shm_name("wrap");
        let ring = SharedRingBuffer::create(&shm_name, 8).unwrap();

        ring.write(b"12345678"); // 填满
        assert_eq!(ring.len(), 8);
        assert_eq!(ring.dump(), b"12345678");

        ring.write(b"ab"); // 回绕覆盖前 2 字节
        assert_eq!(ring.len(), 8);
        assert_eq!(ring.dump(), b"345678ab");

        ring.unlink().unwrap();
    }

    #[test]
    fn test_overflow_write() {
        let shm_name = test_shm_name("overflow");
        let ring = SharedRingBuffer::create(&shm_name, 4).unwrap();

        ring.write(b"abcdefgh"); // 写入 > capacity
        assert_eq!(ring.len(), 4);
        assert_eq!(ring.dump(), b"efgh");

        ring.unlink().unwrap();
    }

    #[test]
    fn test_dump_with_reset() {
        let shm_name = test_shm_name("reset");
        let ring = SharedRingBuffer::create(&shm_name, 16).unwrap();

        ring.write(b"test");
        let data = ring.dump_with_reset();
        assert_eq!(&data[..4], b"\x1b[!p");
        assert_eq!(&data[4..], b"test");

        ring.unlink().unwrap();
    }

    #[test]
    fn test_empty() {
        let shm_name = test_shm_name("empty");
        let ring = SharedRingBuffer::create(&shm_name, 16).unwrap();

        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
        assert!(ring.dump().is_empty());
        assert!(ring.dump_with_reset().is_empty());

        ring.unlink().unwrap();
    }

    #[test]
    fn test_clear() {
        let shm_name = test_shm_name("clear");
        let ring = SharedRingBuffer::create(&shm_name, 16).unwrap();

        ring.write(b"data");
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.dump(), b"");

        ring.unlink().unwrap();
    }

    #[test]
    fn test_cross_mapping() {
        let shm_name = test_shm_name("cross");
        let ring1 = SharedRingBuffer::create(&shm_name, 32).unwrap();

        ring1.write(b"shared data");

        // 第二个进程打开相同的共享内存
        let ring2 = SharedRingBuffer::open(&shm_name).unwrap();
        assert_eq!(ring2.dump(), b"shared data");
        assert_eq!(ring2.len(), 11);

        ring1.unlink().unwrap();
    }

    #[test]
    fn test_magic_validation() {
        let shm_name = test_shm_name("magic");
        let ring = SharedRingBuffer::create(&shm_name, 16).unwrap();

        ring.write(b"test");

        // 破坏 magic
        unsafe {
            std::ptr::write(ring.mmap_ptr as *mut u32, 0xDEADBEEF);
        }

        // dump 应该返回空（检测到 magic 错误）
        assert!(ring.dump().is_empty());

        ring.unlink().unwrap();
    }
}
