//! PTY 创建模块 — raw libc，不依赖 teletypewriter crate

use std::collections::HashMap;
use std::ffi::CStr;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

// macOS TIOCSWINSZ
#[cfg(target_os = "macos")]
const TIOCSWINSZ: libc::c_ulong = 2148037735;

#[link(name = "util")]
extern "C" {
    fn openpty(
        main: *mut libc::c_int,
        child: *mut libc::c_int,
        name: *mut libc::c_char,
        termp: *const libc::termios,
        winsize: *const libc::winsize,
    ) -> libc::c_int;

    fn ptsname(fd: libc::c_int) -> *mut libc::c_char;
}

/// PTY master/child fd pair + child process info
pub struct PtyPair {
    pub master_fd: RawFd,
    pub child_pid: libc::pid_t,
    pub ptsname: String,
}

impl Drop for PtyPair {
    fn drop(&mut self) {
        // 只关 master fd，不杀子进程——daemon 的 session 管理负责生命周期
        unsafe {
            libc::close(self.master_fd);
        }
    }
}

/// 创建 termios 配置（与 teletypewriter 保持一致）
fn create_termios() -> libc::termios {
    let mut term: libc::termios = unsafe { std::mem::zeroed() };

    term.c_iflag = libc::ICRNL | libc::IXON | libc::IXANY | libc::IMAXBEL | libc::BRKINT;
    term.c_oflag = libc::OPOST | libc::ONLCR;
    term.c_cflag = libc::CREAD | libc::CS8 | libc::HUPCL;
    term.c_lflag = libc::ICANON
        | libc::ISIG
        | libc::IEXTEN
        | libc::ECHO
        | libc::ECHOE
        | libc::ECHOK
        | libc::ECHOKE
        | libc::ECHOCTL;

    // UTF-8
    #[cfg(not(target_os = "freebsd"))]
    {
        term.c_iflag |= libc::IUTF8;
    }

    // 标准控制字符
    term.c_cc[libc::VEOF] = 4;
    term.c_cc[libc::VEOL] = 255;
    term.c_cc[libc::VEOL2] = 255;
    term.c_cc[libc::VERASE] = 0x7f;
    term.c_cc[libc::VWERASE] = 23;
    term.c_cc[libc::VKILL] = 21;
    term.c_cc[libc::VREPRINT] = 18;
    term.c_cc[libc::VINTR] = 3;
    term.c_cc[libc::VQUIT] = 0x1c;
    term.c_cc[libc::VSUSP] = 26;
    term.c_cc[libc::VSTART] = 17;
    term.c_cc[libc::VSTOP] = 19;
    term.c_cc[libc::VLNEXT] = 22;
    term.c_cc[libc::VDISCARD] = 15;
    term.c_cc[libc::VMIN] = 1;
    term.c_cc[libc::VTIME] = 0;

    #[cfg(target_os = "macos")]
    {
        term.c_cc[libc::VDSUSP] = 25;
        term.c_cc[libc::VSTATUS] = 20;
    }

    term
}

/// 设置 fd 为非阻塞
unsafe fn set_nonblocking(fd: RawFd) {
    let flags = libc::fcntl(fd, libc::F_GETFL, 0);
    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
}

/// 获取 pts name
fn get_ptsname(fd: RawFd) -> String {
    unsafe {
        let ptr = ptsname(fd);
        if ptr.is_null() {
            return String::new();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

/// 创建 PTY 并 spawn shell 进程
///
/// daemon 创建 PTY 后持有 master fd，shell 进程在 child 端运行。
/// 与 teletypewriter 的关键区别：Child Drop 不发 SIGHUP，
/// 生命周期由 session manager 控制。
pub fn create_pty(
    shell: &str,
    cols: u16,
    rows: u16,
    working_dir: Option<&str>,
    terminal_id: Option<u32>,
    envs: Option<&HashMap<String, String>>,
) -> io::Result<PtyPair> {
    let mut master: libc::c_int = 0;
    let mut child: libc::c_int = 0;

    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let term = create_termios();

    let res = unsafe {
        openpty(
            &mut master,
            &mut child,
            std::ptr::null_mut(),
            &term,
            &winsize,
        )
    };
    if res < 0 {
        return Err(io::Error::last_os_error());
    }

    // child fd 交给子进程的 stdin/stdout/stderr
    let owned_child = unsafe { OwnedFd::from_raw_fd(child) };

    let shell_program = if shell.is_empty() {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    } else {
        shell.to_string()
    };

    let mut builder = Command::new(&shell_program);
    builder.arg("--login");

    // stdin/stdout/stderr → child fd
    builder.stdin(owned_child.try_clone()?);
    builder.stderr(owned_child.try_clone()?);
    builder.stdout(owned_child);

    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("LC_CTYPE", "UTF-8");
    builder.env("LANG", "en_US.UTF-8");

    builder.env("ETERM_SHELL_INTEGRATION", "1");
    builder.env("TERM_PROGRAM", "ETerm");

    if let Some(tid) = terminal_id {
        let tid_str = tid.to_string();
        builder.env("ETERM_TERMINAL_ID", &tid_str);
        builder.env("ETERM_SESSION_ID", &tid_str);
    }

    // Apply extra environment variables from client (ZDOTDIR, ETERM_SHELL_DIR, etc.)
    if let Some(extra_envs) = envs {
        for (key, value) in extra_envs {
            builder.env(key, value);
        }
    }

    if let Some(dir) = working_dir {
        let path = std::path::Path::new(dir);
        if path.is_absolute() && path.exists() && path.is_dir() {
            builder.current_dir(dir);
        }
    }

    // pre_exec: setsid + set controlling terminal
    unsafe {
        let child_fd = child;
        let master_fd = master;
        builder.pre_exec(move || {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }

            // TIOCSCTTY — set controlling terminal
            #[cfg(target_os = "macos")]
            {
                if libc::ioctl(child_fd, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }

            libc::close(child_fd);
            libc::close(master_fd);

            libc::signal(libc::SIGCHLD, libc::SIG_DFL);
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGALRM, libc::SIG_DFL);

            Ok(())
        });
    }

    let child_process = builder.spawn().map_err(|e| {
        // spawn 失败要关 master fd
        unsafe { libc::close(master); }
        e
    })?;

    let pid = child_process.id() as libc::pid_t;
    let ptsname = get_ptsname(master);

    unsafe {
        set_nonblocking(master);
    }

    // 注意：我们故意不持有 child_process（让它 detach）
    // daemon 用 kqueue EVFILT_PROC 监听 pid，不靠 Child::wait
    std::mem::forget(child_process);

    Ok(PtyPair {
        master_fd: master,
        child_pid: pid,
        ptsname,
    })
}

/// 设置 PTY 窗口大小
pub fn set_winsize(fd: RawFd, cols: u16, rows: u16) -> io::Result<()> {
    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let res = unsafe { libc::ioctl(fd, TIOCSWINSZ, &winsize) };
    if res < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
