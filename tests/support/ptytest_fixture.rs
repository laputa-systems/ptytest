//! A deterministic real-TTY fixture used only to prove `ptytest` contracts.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static WINCH: AtomicBool = AtomicBool::new(false);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static DESCENDANT_TERMINATED: AtomicBool = AtomicBool::new(false);

extern "C" fn record_winch(_: libc::c_int) {
    WINCH.store(true, Ordering::Relaxed);
}
extern "C" fn record_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Relaxed);
}
extern "C" fn record_descendant_termination(_: libc::c_int) {
    DESCENDANT_TERMINATED.store(true, Ordering::Relaxed);
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--wait-sigint") => return wait_for_sigint(),
        Some("--wait-eof") => return wait_for_eof(),
        Some("--leader-exits-with-descendant") => return leader_exits_with_descendant(),
        _ => {}
    }
    install_signal(libc::SIGWINCH, record_winch);
    set_raw_stdin();
    line("\x1b[2J\x1b[HPTYTEST_READY");
    loop {
        let command = read_line();
        match command.as_deref() {
            Some("ownership") => report_ownership(),
            Some("size") => report_size(),
            Some("winch") => {
                line("RESIZE_WAIT");
                while !WINCH.swap(false, Ordering::Relaxed) {
                    pause();
                }
                line("RESIZE_SIGNAL");
            }
            Some("input") => line("INPUT_ACK"),
            Some("alternate-on") => line("\x1b[?1049hALT_ON"),
            Some("alternate-off") => line("\x1b[?1049lALT_OFF"),
            Some("application-cursor") => line("\x1b[?1hAPP_CURSOR_ON"),
            Some("normal-cursor") => line("\x1b[?1lAPP_CURSOR_OFF"),
            Some("key") => {
                line("KEY_READY");
                let bytes = read_terminal_key();
                line(&format!("KEY:{}", escaped(&bytes)));
            }
            Some("query-cpr") => {
                write_all(b"\x1b[6");
                write_all(b"n");
                let reply = read_until(b'R');
                line(&format!("QUERY_REPLY:{}", escaped(&reply)));
            }
            Some("query-cpr-with-trailing-output") => {
                // The reply must use the cursor after `A`, before `BC`.
                write_all(b"A\x1b[6nBC");
                let reply = read_until(b'R');
                line(&format!("QUERY_CPR_REPLY:{}", escaped(&reply)));
            }
            Some("split-output") => {
                write_all(b"\x1b[31msp");
                write_all("lit-界".as_bytes());
                line("\x1b[0m");
            }
            Some("unicode") => {
                line("UNICODE_READY");
                let bytes = read_exact(3);
                line(&format!("UNICODE:{}", String::from_utf8_lossy(&bytes)));
            }
            Some("backpressure") => {
                line("BACKPRESSURE_READY");
                let bytes = read_exact(512 * 1024);
                line(&format!("BACKPRESSURE_READ={}", bytes.len()));
            }
            Some("descendant") => spawn_descendant(),
            Some(command) if command.starts_with("exit:") => {
                let code = command[5..].parse::<i32>().unwrap_or(127);
                line("FIXTURE_DONE");
                std::process::exit(code);
            }
            Some(_) => line("FIXTURE_UNKNOWN"),
            None => return,
        }
    }
}

fn wait_for_sigint() {
    install_signal(libc::SIGINT, record_interrupt);
    line("SIGINT_READY");
    while !INTERRUPTED.load(Ordering::Relaxed) {
        pause();
    }
    line("SIGINT_RECEIVED");
}

fn wait_for_eof() {
    line("EOF_READY");
    let mut byte = [0_u8; 1];
    let count = unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), 1) };
    if count == 0 {
        line("EOF_RECEIVED");
    }
}

fn leader_exits_with_descendant() {
    let mut readiness = [-1; 2];
    if unsafe { libc::pipe(readiness.as_mut_ptr()) } != 0 {
        line("DESCENDANT_PIPE_FAILED");
        std::process::exit(127);
    }
    let child = unsafe { libc::fork() };
    if child == 0 {
        unsafe { libc::close(readiness[0]) };
        // Session-leader exit normally sends SIGHUP to the foreground group;
        // the regression needs the ordinary descendant to remain alive until
        // `PtyTest::finish` targets that original group.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
        install_signal(libc::SIGTERM, record_descendant_termination);
        line(&format!("DESCENDANT_PID={}", unsafe { libc::getpid() }));
        line("DESCENDANT_READY");
        let ready = [1_u8];
        let wrote = unsafe { libc::write(readiness[1], ready.as_ptr().cast(), ready.len()) };
        unsafe { libc::close(readiness[1]) };
        if wrote != 1 {
            std::process::exit(127);
        }
        loop {
            pause();
            if DESCENDANT_TERMINATED.swap(false, Ordering::Relaxed) {
                // The session leader's exit can revoke the controlling TTY,
                // so this marker is a reliable termination witness even when
                // stdout can no longer be observed from the master.
                let marker = std::env::temp_dir().join("ptytest-descendant-terminated");
                std::fs::write(marker, b"terminated").expect("write descendant termination marker");
                std::process::exit(0);
            }
        }
    }
    if child > 0 {
        unsafe { libc::close(readiness[1]) };
        let mut ready = [0_u8];
        let read = unsafe { libc::read(readiness[0], ready.as_mut_ptr().cast(), ready.len()) };
        unsafe { libc::close(readiness[0]) };
        if read != 1 {
            line("DESCENDANT_SETUP_FAILED");
            std::process::exit(127);
        }
        line("LEADER_EXITING");
        std::process::exit(0);
    }
    unsafe {
        libc::close(readiness[0]);
        libc::close(readiness[1]);
    }
    line("DESCENDANT_FORK_FAILED");
    std::process::exit(127);
}

fn spawn_descendant() {
    let child = unsafe { libc::fork() };
    if child == 0 {
        loop {
            pause();
        }
    }
    if child > 0 {
        line("DESCENDANT_READY");
        loop {
            pause();
        }
    }
    line("DESCENDANT_FORK_FAILED");
}

fn report_ownership() {
    let pid = unsafe { libc::getpid() };
    let sid = unsafe { libc::getsid(0) };
    let process_group = unsafe { libc::getpgrp() };
    let foreground_group = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
    let terminal_session = unsafe { libc::tcgetsid(libc::STDIN_FILENO) };
    let tty = unsafe {
        libc::isatty(libc::STDIN_FILENO) == 1
            && libc::isatty(libc::STDOUT_FILENO) == 1
            && libc::isatty(libc::STDERR_FILENO) == 1
    };
    line(
        if tty
            && sid == pid
            && process_group == pid
            && foreground_group == pid
            && terminal_session == pid
        {
            "OWNERSHIP_OK"
        } else {
            "OWNERSHIP_BAD"
        },
    );
}

fn report_size() {
    let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
    let result = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) };
    if result == 0 {
        line(&format!(
            "SIZE rows={} columns={}",
            size.ws_row, size.ws_col
        ));
    } else {
        line("SIZE_ERROR");
    }
}

fn set_raw_stdin() {
    let mut settings = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut settings) } != 0 {
        return;
    }
    unsafe { libc::cfmakeraw(&mut settings) };
    settings.c_cc[libc::VMIN] = 1;
    settings.c_cc[libc::VTIME] = 0;
    unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &settings) };
}

fn install_signal(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    unsafe { libc::signal(signal, handler as libc::sighandler_t) };
}

fn pause() {
    unsafe { libc::pause() };
}

fn read_line() -> Option<String> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = 0_u8;
        let count = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
        if count == 0 {
            return None;
        }
        if count < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return None;
        }
        if matches!(byte, b'\r' | b'\n') {
            return String::from_utf8(bytes).ok();
        }
        bytes.push(byte);
    }
}

fn read_terminal_key() -> Vec<u8> {
    let first = read_byte();
    if first != 0x1b {
        return vec![first];
    }
    let second = read_byte();
    if !matches!(second, b'[' | b'O') {
        return vec![first, second];
    }
    vec![first, second, read_byte()]
}

fn read_until(final_byte: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = 0_u8;
        if unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) } == 1 {
            bytes.push(byte);
            if byte == final_byte {
                return bytes;
            }
        }
    }
}

fn read_byte() -> u8 {
    loop {
        let mut byte = 0_u8;
        let count = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
        if count == 1 {
            return byte;
        }
        if count < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        std::process::exit(127);
    }
}

fn read_exact(length: usize) -> Vec<u8> {
    let mut bytes = vec![0_u8; length];
    let mut offset = 0;
    while offset < bytes.len() {
        let count = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                bytes[offset..].as_mut_ptr().cast(),
                bytes.len() - offset,
            )
        };
        if count > 0 {
            offset += count as usize;
        } else if count < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        } else {
            bytes.truncate(offset);
            return bytes;
        }
    }
    bytes
}

fn write_all(bytes: &[u8]) {
    let mut stdout = io::stdout().lock();
    stdout.write_all(bytes).unwrap();
    stdout.flush().unwrap();
}

fn line(text: &str) {
    write_all(text.as_bytes());
    write_all(b"\r\n");
}

fn escaped(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| std::ascii::escape_default(*byte))
        .map(char::from)
        .collect()
}
