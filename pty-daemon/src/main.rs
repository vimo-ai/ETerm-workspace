//! pty-daemon CLI
//!
//! 用法:
//!   pty-daemon daemon              启动 daemon
//!   pty-daemon create [--shell X]  创建 session
//!   pty-daemon attach <id>         attach 到 session
//!   pty-daemon detach <id>         detach session
//!   pty-daemon list                列出所有 session
//!   pty-daemon kill <id>           杀掉 session
//!   pty-daemon ping                健康检查
//!   pty-daemon shutdown            优雅关闭 daemon

use pty_daemon::fd_passing;
use pty_daemon::protocol::{self, Request, Response};
use pty_daemon::server::Server;
use pty_daemon::shared_ring::SharedRingBuffer;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use uuid::Uuid;

const DEFAULT_SOCKET: &str = "/tmp/eterm-daemon.sock";

fn socket_path() -> PathBuf {
    std::env::var("PTY_DAEMON_SOCK")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_SOCKET))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let result = match args[1].as_str() {
        "daemon" => cmd_daemon(),
        "create" => cmd_create(&args[2..]),
        "attach" => cmd_attach(&args[2..]),
        "detach" => cmd_detach(&args[2..]),
        "list" => cmd_list(),
        "kill" => cmd_kill(&args[2..]),
        "ping" => cmd_ping(),
        "shutdown" => cmd_shutdown(),
        _ => {
            print_usage();
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!(
        "pty-daemon v0.1.0

Usage:
  pty-daemon daemon              Start daemon
  pty-daemon create [--shell X]  Create session
  pty-daemon attach <id>         Attach to session
  pty-daemon detach <id>         Detach session
  pty-daemon list                List sessions
  pty-daemon kill <id>           Kill session
  pty-daemon ping                Health check
  pty-daemon shutdown            Gracefully shutdown daemon"
    );
}

// ============================================================================
// daemon 模式
// ============================================================================

fn cmd_daemon() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("pty-daemon v0.1.0 (pid={})", std::process::id());

    let path = socket_path();
    let mut server = Server::new(&path)?;
    server.run()?;
    Ok(())
}

// ============================================================================
// 客户端命令
// ============================================================================

fn connect() -> Result<UnixStream, Box<dyn std::error::Error>> {
    let path = socket_path();
    let stream = UnixStream::connect(&path).map_err(|e| {
        format!(
            "cannot connect to daemon at {}: {e}\nIs the daemon running?",
            path.display()
        )
    })?;
    Ok(stream)
}

fn send_request(
    stream: &mut UnixStream,
    req: &Request,
) -> Result<Response, Box<dyn std::error::Error>> {
    let encoded = protocol::encode_message(req);
    stream.write_all(&encoded)?;

    // 读响应
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            return Err("daemon closed connection".into());
        }
        buf.extend_from_slice(&tmp[..n]);

        if let Some((result, _)) = protocol::try_decode_message::<Response>(&buf) {
            return result.map_err(|e| e.into());
        }
    }
}

fn cmd_create(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut shell = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--shell" => {
                i += 1;
                shell = args.get(i).cloned();
            }
            _ => {}
        }
        i += 1;
    }

    let mut stream = connect()?;
    let resp = send_request(
        &mut stream,
        &Request::Create {
            shell,
            cols: 80,
            rows: 24,
            working_dir: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            terminal_id: None,
            envs: None,
        },
    )?;

    match resp {
        Response::Created { session_id } => {
            println!("{session_id}");
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        _ => eprintln!("unexpected response: {resp:?}"),
    }
    Ok(())
}

fn cmd_attach(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        eprintln!("usage: pty-daemon attach <session-id>");
        std::process::exit(1);
    }

    let id: Uuid = args[0].parse().map_err(|_| "invalid uuid")?;
    let mut stream = connect()?;
    let resp = send_request(&mut stream, &Request::Attach { session_id: id })?;

    match resp {
        Response::AttachReady {
            session_id,
            cols,
            rows,
            child_pid,
            shm_name,
        } => {
            // 接收 fd
            use std::os::unix::io::AsRawFd;
            let master_fd = fd_passing::recv_fd(stream.as_raw_fd())?;

            // 打开共享内存 ring buffer，直接从 shm 读取历史数据并 replay
            let shared_ring = SharedRingBuffer::open(&shm_name)
                .map_err(|e| format!("failed to open shared ring {shm_name}: {e}"))?;
            let ring_data = shared_ring.dump();
            if !ring_data.is_empty() {
                std::io::stdout().write_all(&ring_data)?;
            }

            println!("attached to {session_id}");
            println!(
                "  master_fd={master_fd} child_pid={child_pid} size={cols}x{rows} shm={shm_name}"
            );

            // 简易交互：stdin → PTY，PTY → stdout + 共享内存
            run_interactive(master_fd, id, &mut stream, &shared_ring)?;
        }
        Response::AttachDeny { reason, .. } => {
            eprintln!("attach denied: {reason}");
            std::process::exit(1);
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        _ => eprintln!("unexpected response: {resp:?}"),
    }

    Ok(())
}

/// 简易交互模式：raw terminal + fd 双向转发 + 共享内存写入
fn run_interactive(
    master_fd: i32,
    session_id: Uuid,
    control_stream: &mut UnixStream,
    shared_ring: &SharedRingBuffer,
) -> Result<(), Box<dyn std::error::Error>> {
    // 设置 stdin 为 raw mode
    let mut old_termios: libc::termios = unsafe { std::mem::zeroed() };
    unsafe {
        libc::tcgetattr(0, &mut old_termios);
    }
    let mut raw = old_termios;
    unsafe {
        libc::cfmakeraw(&mut raw);
    }
    unsafe {
        libc::tcsetattr(0, libc::TCSANOW, &raw);
    }

    // 用 poll 做双向转发
    let mut poll_fds = [
        libc::pollfd {
            fd: 0, // stdin
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: master_fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];

    let mut buf = [0u8; 4096];
    let mut running = true;

    while running {
        let ret = unsafe { libc::poll(poll_fds.as_mut_ptr(), 2, 100) };
        if ret < 0 {
            break;
        }

        // stdin → PTY
        if poll_fds[0].revents & libc::POLLIN != 0 {
            let n = unsafe { libc::read(0, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                running = false;
            } else {
                // Ctrl+] (0x1d) = detach
                if buf[..n as usize].contains(&0x1d) {
                    eprintln!("\r\n[detached from {session_id}]");
                    // 发 detach 请求
                    let _ = send_request(
                        control_stream,
                        &Request::Detach {
                            session_id,
                            cols: 80,
                            rows: 24,
                        },
                    );
                    running = false;
                } else {
                    unsafe {
                        libc::write(master_fd, buf.as_ptr() as *const _, n as usize);
                    }
                }
            }
        }

        // PTY → stdout + 共享内存
        if poll_fds[1].revents & libc::POLLIN != 0 {
            let n = unsafe { libc::read(master_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                running = false;
            } else {
                let data = &buf[..n as usize];
                unsafe {
                    libc::write(1, data.as_ptr() as *const _, data.len());
                }
                // 写入共享内存 ring buffer（~10-50ns memcpy，不影响热路径）
                shared_ring.write(data);
            }
        }

        // EOF / error
        if poll_fds[1].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            running = false;
        }
    }

    // 恢复 termios
    unsafe {
        libc::tcsetattr(0, libc::TCSANOW, &old_termios);
    }
    unsafe {
        libc::close(master_fd);
    }

    Ok(())
}

fn cmd_detach(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        eprintln!("usage: pty-daemon detach <session-id>");
        std::process::exit(1);
    }

    let id: Uuid = args[0].parse().map_err(|_| "invalid uuid")?;
    let mut stream = connect()?;
    let resp = send_request(
        &mut stream,
        &Request::Detach {
            session_id: id,
            cols: 80,
            rows: 24,
        },
    )?;

    match resp {
        Response::Detached { session_id } => println!("detached {session_id}"),
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        _ => eprintln!("unexpected response: {resp:?}"),
    }
    Ok(())
}

fn cmd_list() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = connect()?;
    let resp = send_request(&mut stream, &Request::List)?;

    match resp {
        Response::SessionList { sessions } => {
            if sessions.is_empty() {
                println!("no sessions");
            } else {
                println!(
                    "{:<36}  {:<10}  {:<8}  {:<10}  {:<6}  {}",
                    "ID", "STATE", "PID", "SIZE", "ALIVE", "AGE"
                );
                for s in sessions {
                    println!(
                        "{:<36}  {:<10}  {:<8}  {}x{:<6}  {:<6}  {}s",
                        s.id,
                        s.state,
                        s.child_pid,
                        s.cols,
                        s.rows,
                        s.child_alive,
                        s.created_secs_ago
                    );
                }
            }
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        _ => eprintln!("unexpected response: {resp:?}"),
    }
    Ok(())
}

fn cmd_kill(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        eprintln!("usage: pty-daemon kill <session-id>");
        std::process::exit(1);
    }

    let id: Uuid = args[0].parse().map_err(|_| "invalid uuid")?;
    let mut stream = connect()?;
    let resp = send_request(&mut stream, &Request::Kill { session_id: id })?;

    match resp {
        Response::Killed { session_id } => println!("killed {session_id}"),
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        _ => eprintln!("unexpected response: {resp:?}"),
    }
    Ok(())
}

fn cmd_ping() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = connect()?;
    let resp = send_request(&mut stream, &Request::Ping)?;

    match resp {
        Response::Pong {
            version,
            session_count,
        } => {
            println!("pong v{version} sessions={session_count}");
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        _ => eprintln!("unexpected response: {resp:?}"),
    }
    Ok(())
}

fn cmd_shutdown() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = connect()?;
    let resp = send_request(&mut stream, &Request::Shutdown)?;

    match resp {
        Response::ShuttingDown => {
            println!("daemon is shutting down");
        }
        Response::Error { message } => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        _ => eprintln!("unexpected response: {resp:?}"),
    }
    Ok(())
}
