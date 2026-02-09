//! fd passing via SCM_RIGHTS (sendmsg/recvmsg)
//!
//! Attach 时 daemon 将 PTY master fd 传给客户端，
//! 实现零开销的 fd 交接。

use std::io;
use std::os::fd::RawFd;

/// 通过 Unix domain socket 发送 fd
///
/// 利用 sendmsg + SCM_RIGHTS ancillary data 传递 fd。
/// 同时发送 1 字节 payload（协议要求至少 1 字节 iov）。
pub fn send_fd(socket: RawFd, fd: RawFd) -> io::Result<()> {
    // cmsg buffer 大小：CMSG_SPACE(sizeof(int))
    let cmsg_size = unsafe { libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_size];

    // 至少要有 1 字节 payload
    let payload = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: payload.as_ptr() as *mut _,
        iov_len: 1,
    };

    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut _;
    msg.msg_controllen = cmsg_size as _;

    // 填充 cmsg header
    let cmsg: *mut libc::cmsghdr = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    unsafe {
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as u32) as _;

        // 写入 fd
        let fd_ptr = libc::CMSG_DATA(cmsg) as *mut libc::c_int;
        *fd_ptr = fd;
    }

    let ret = unsafe { libc::sendmsg(socket, &msg, 0) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// 通过 Unix domain socket 接收 fd
///
/// 返回接收到的 fd。调用方负责关闭。
pub fn recv_fd(socket: RawFd) -> io::Result<RawFd> {
    let cmsg_size = unsafe { libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as u32) } as usize;
    let mut cmsg_buf = vec![0u8; cmsg_size];

    let mut payload = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: payload.as_mut_ptr() as *mut _,
        iov_len: 1,
    };

    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buf.as_mut_ptr() as *mut _;
    msg.msg_controllen = cmsg_size as _;

    let ret = unsafe { libc::recvmsg(socket, &mut msg, 0) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    if ret == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed"));
    }

    // 从 ancillary data 中提取 fd
    let cmsg = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    if cmsg.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no ancillary data",
        ));
    }

    unsafe {
        if (*cmsg).cmsg_level != libc::SOL_SOCKET || (*cmsg).cmsg_type != libc::SCM_RIGHTS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected cmsg type",
            ));
        }

        let fd_ptr = libc::CMSG_DATA(cmsg) as *const libc::c_int;
        Ok(*fd_ptr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fd_passing_roundtrip() {
        // 创建 socketpair
        let mut fds = [0i32; 2];
        let ret = unsafe {
            libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr())
        };
        assert_eq!(ret, 0);

        // 创建一个临时 fd（用 pipe）来传递
        let mut pipe_fds = [0i32; 2];
        let ret = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
        assert_eq!(ret, 0);

        let pipe_read = pipe_fds[0];
        let pipe_write = pipe_fds[1];

        // 先写数据到 pipe
        unsafe {
            libc::write(pipe_write, b"hello".as_ptr() as *const _, 5);
        }

        // 通过 socketpair 传递 pipe_read fd
        send_fd(fds[0], pipe_read).unwrap();

        // 传完后关掉原始的 pipe_read
        unsafe { libc::close(pipe_read); }

        // 在另一端接收 fd
        let received_fd = recv_fd(fds[1]).unwrap();

        // 验证收到的 fd 可以读到数据
        let mut buf = [0u8; 16];
        let n = unsafe {
            libc::read(received_fd, buf.as_mut_ptr() as *mut _, buf.len())
        };
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], b"hello");

        // 清理
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
            libc::close(pipe_write);
            libc::close(received_fd);
        }
    }
}
