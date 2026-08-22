//! The only module that owns Unix PTY/process details.

use crate::config::{CommandSpec, ScenarioParts, Size, TestEnv};
use crate::io::ExitStatus;
use crate::{PtyTestError, Result};
use std::collections::BTreeMap;
use std::ffi::{CString, OsStr, OsString};
#[cfg(target_os = "linux")]
use std::fs;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};

// `openpty` is in libutil on Linux and in libc on macOS. Keeping this linker
// detail here prevents platform C ABI knowledge from leaking into the crate.
#[cfg(target_os = "linux")]
#[link(name = "util")]
unsafe extern "C" {}

// Darwin can reject `kill(-pgid, signal)` with EPERM once the session leader
// is a zombie, even if ordinary descendants remain in that original group.
// `libproc` exposes current process identifiers; the narrow fallback below
// rechecks each candidate's group immediately before signalling it.
#[cfg(target_os = "macos")]
#[link(name = "proc")]
unsafe extern "C" {
    fn proc_listallpids(buffer: *mut libc::c_void, buffersize: libc::c_int) -> libc::c_int;
}

pub(crate) struct Spawned {
    pub(crate) master: OwnedFd,
    pub(crate) child_pid: libc::pid_t,
    pub(crate) process_group: libc::pid_t,
}

pub(crate) fn spawn(scenario: &ScenarioParts) -> Result<Spawned> {
    let prepared = PreparedExec::new(&scenario.command, &scenario.environment)?;
    let (master, slave) = open_pty(scenario.size)?;
    let (error_read, error_write) = error_pipe()?;
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(system("fork"));
    }
    if pid == 0 {
        unsafe {
            child_exec(
                master.as_raw_fd(),
                slave.as_raw_fd(),
                error_write.as_raw_fd(),
                &prepared,
            )
        }
    }

    drop(slave);
    drop(error_write);
    let setup_error = read_child_error(error_read.as_raw_fd())?;
    drop(error_read);
    if let Some((stage, errno)) = setup_error {
        reap_rejected_spawn(pid);
        return Err(PtyTestError::SpawnFailed {
            stage: stage_name(stage),
            errno,
        });
    }
    set_nonblocking(master.as_raw_fd())?;
    Ok(Spawned {
        master,
        child_pid: pid,
        process_group: pid,
    })
}

pub(crate) fn resize(master: RawFd, size: Size) -> Result<()> {
    let window_size = libc::winsize {
        ws_row: size.rows(),
        ws_col: size.columns(),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let result = unsafe { libc::ioctl(master, libc::TIOCSWINSZ, &window_size) };
    if result < 0 {
        Err(system("TIOCSWINSZ"))
    } else {
        Ok(())
    }
}

pub(crate) fn terminal_eof(master: RawFd) -> Result<u8> {
    let mut termios = MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(master, termios.as_mut_ptr()) } < 0 {
        return Err(system("tcgetattr"));
    }
    let termios = unsafe { termios.assume_init() };
    if termios.c_lflag & libc::ICANON == 0 {
        return Err(PtyTestError::System {
            operation: "send_eof requires canonical line discipline",
            errno: libc::EINVAL,
        });
    }
    Ok(termios.c_cc[libc::VEOF])
}

pub(crate) fn signal_group(process_group: libc::pid_t, signal: libc::c_int) -> Result<()> {
    if signal <= 0 {
        return Err(PtyTestError::InvalidSignal { signal });
    }
    if unsafe { libc::kill(-process_group, signal) } < 0 {
        let errno = errno();
        #[cfg(target_os = "macos")]
        if errno == libc::EPERM {
            return signal_darwin_group_members(process_group, signal);
        }
        if errno != libc::ESRCH {
            return Err(PtyTestError::System {
                operation: "kill process group",
                errno,
            });
        }
    }
    Ok(())
}

/// Reports whether an unreaped original group still contains a running member
/// other than its leader. This is deliberately platform-local: Linux exposes
/// process state through procfs, while Darwin exposes current group membership
/// through libproc. A zombie leader alone is not a reason to escalate.
pub(crate) fn group_has_live_member(
    process_group: libc::pid_t,
    leader: libc::pid_t,
) -> Result<bool> {
    #[cfg(target_os = "linux")]
    return linux_group_has_live_member(process_group, leader);
    #[cfg(target_os = "macos")]
    return Ok(darwin_group_members(process_group)?
        .into_iter()
        .any(|pid| pid != leader && unsafe { libc::getpgid(pid) } == process_group));
    #[allow(unreachable_code)]
    Err(PtyTestError::System {
        operation: "inspect process group on unsupported Unix",
        errno: libc::ENOTSUP,
    })
}

#[cfg(target_os = "linux")]
fn linux_group_has_live_member(process_group: libc::pid_t, leader: libc::pid_t) -> Result<bool> {
    for entry in
        fs::read_dir("/proc").map_err(|error| PtyTestError::io_at("read procfs", "/proc", error))?
    {
        let entry =
            entry.map_err(|error| PtyTestError::io_at("read procfs entry", "/proc", error))?;
        let Some(name) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if name == leader {
            continue;
        }
        let stat_path = entry.path().join("stat");
        let stat = match fs::read_to_string(&stat_path) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(PtyTestError::io_at("read procfs stat", stat_path, error)),
        };
        let Some((_, fields)) = stat.rsplit_once(") ") else {
            continue;
        };
        let mut fields = fields.split_ascii_whitespace();
        let state = fields.next();
        let _parent = fields.next();
        let group = fields
            .next()
            .and_then(|value| value.parse::<libc::pid_t>().ok());
        if group == Some(process_group) && state != Some("Z") {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn signal_darwin_group_members(process_group: libc::pid_t, signal: libc::c_int) -> Result<()> {
    let members = darwin_group_members(process_group)?;
    for pid in members {
        // The original leader remains unreaped during cleanup, so this group
        // ID cannot be recycled. Verify membership immediately before the
        // targeted fallback signal; an unrelated reused PID cannot join this
        // session's group, while a newly joined same-session process remains
        // in the cleanup contract.
        if unsafe { libc::getpgid(pid) } != process_group {
            continue;
        }
        if unsafe { libc::kill(pid, signal) } < 0 {
            let errno = errno();
            if errno != libc::ESRCH {
                return Err(PtyTestError::System {
                    operation: "kill Darwin process-group member",
                    errno,
                });
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn darwin_group_members(process_group: libc::pid_t) -> Result<Vec<libc::pid_t>> {
    let byte_count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
    if byte_count < 0 {
        return Err(system("proc_listallpids size"));
    }
    if byte_count == 0 {
        return Ok(Vec::new());
    }
    let pid_bytes = std::mem::size_of::<libc::pid_t>();
    let slots = (byte_count as usize).div_ceil(pid_bytes).saturating_add(8);
    let mut members = vec![0; slots];
    let written = unsafe {
        proc_listallpids(
            members.as_mut_ptr().cast(),
            (members.len() * pid_bytes) as libc::c_int,
        )
    };
    if written < 0 {
        return Err(system("proc_listallpids members"));
    }
    members.truncate((written as usize) / pid_bytes);
    members.retain(|pid| *pid > 0 && unsafe { libc::getpgid(*pid) } == process_group);
    Ok(members)
}

/// Observes an exit without consuming the child's wait status.
pub(crate) fn observe_exit(pid: libc::pid_t) -> Result<Option<ExitStatus>> {
    let mut info = MaybeUninit::<libc::siginfo_t>::zeroed();
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            info.as_mut_ptr(),
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result < 0 {
        let observed_errno = errno();
        if observed_errno == libc::ECHILD {
            return Ok(None);
        }
        return Err(PtyTestError::System {
            operation: "waitid",
            errno: observed_errno,
        });
    }
    let info = unsafe { info.assume_init() };
    if unsafe { info.si_pid() } == 0 {
        return Ok(None);
    }
    let status = unsafe { info.si_status() };
    let exit = match info.si_code {
        libc::CLD_EXITED => ExitStatus::Code(status),
        libc::CLD_KILLED | libc::CLD_DUMPED => ExitStatus::Signal(status),
        _ => return Ok(None),
    };
    Ok(Some(exit))
}

pub(crate) fn reap(pid: libc::pid_t) -> Result<ExitStatus> {
    let mut status = 0;
    loop {
        let result = unsafe { libc::waitpid(pid, &mut status, 0) };
        if result == pid {
            return Ok(exit_status(status));
        }
        if result < 0 && errno() == libc::EINTR {
            continue;
        }
        if result < 0 && errno() == libc::ECHILD {
            return Ok(ExitStatus::Unreaped);
        }
        return Err(system("waitpid"));
    }
}

fn open_pty(size: Size) -> Result<(OwnedFd, OwnedFd)> {
    let mut master = -1;
    let mut slave = -1;
    let mut window_size = libc::winsize {
        ws_row: size.rows(),
        ws_col: size.columns(),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut window_size,
        )
    } < 0
    {
        return Err(system("openpty"));
    }
    let master = unsafe { OwnedFd::from_raw_fd(move_fd_at_least(master, 3)?) };
    let slave = unsafe { OwnedFd::from_raw_fd(move_fd_at_least(slave, 3)?) };
    set_cloexec(master.as_raw_fd())?;
    set_cloexec(slave.as_raw_fd())?;
    Ok((master, slave))
}

fn error_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut descriptors = [-1; 2];
    if unsafe { libc::pipe(descriptors.as_mut_ptr()) } < 0 {
        return Err(system("pipe"));
    }
    let read = unsafe { OwnedFd::from_raw_fd(move_fd_at_least(descriptors[0], 3)?) };
    let write = unsafe { OwnedFd::from_raw_fd(move_fd_at_least(descriptors[1], 3)?) };
    set_cloexec(read.as_raw_fd())?;
    set_cloexec(write.as_raw_fd())?;
    Ok((read, write))
}

fn move_fd_at_least(fd: RawFd, minimum: RawFd) -> Result<RawFd> {
    if fd >= minimum {
        return Ok(fd);
    }
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD, minimum) };
    let close_result = unsafe { libc::close(fd) };
    if duplicate < 0 {
        return Err(system("fcntl F_DUPFD"));
    }
    if close_result < 0 {
        return Err(system("close duplicate source"));
    }
    Ok(duplicate)
}

fn set_cloexec(fd: RawFd) -> Result<()> {
    let current = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if current < 0 {
        return Err(system("fcntl F_GETFD"));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, current | libc::FD_CLOEXEC) } < 0 {
        return Err(system("fcntl F_SETFD"));
    }
    Ok(())
}

pub(crate) fn set_nonblocking(fd: RawFd) -> Result<()> {
    let current = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if current < 0 {
        return Err(system("fcntl F_GETFL"));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, current | libc::O_NONBLOCK) } < 0 {
        return Err(system("fcntl F_SETFL"));
    }
    Ok(())
}

struct PreparedExec {
    program: CString,
    _arguments: Vec<CString>,
    argument_pointers: Vec<*const libc::c_char>,
    _environment: Vec<CString>,
    environment_pointers: Vec<*const libc::c_char>,
    current_dir: Option<CString>,
}

impl PreparedExec {
    fn new(command: &CommandSpec, environment: &TestEnv) -> Result<Self> {
        let program = cstring("command program", command.program().as_os_str())?;
        let mut arguments = Vec::with_capacity(command.arguments().len() + 1);
        arguments.push(program.clone());
        for argument in command.arguments() {
            arguments.push(cstring("command argument", argument)?);
        }
        let mut argument_pointers = arguments
            .iter()
            .map(|argument| argument.as_ptr())
            .collect::<Vec<_>>();
        argument_pointers.push(std::ptr::null());

        let mut variables: BTreeMap<OsString, Option<OsString>> = environment.variables().clone();
        variables.extend(
            command
                .environment()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        let mut values = Vec::new();
        for (key, value) in variables {
            let Some(value) = value else { continue };
            if key.as_bytes().contains(&b'=') {
                return Err(PtyTestError::InteriorNul {
                    field: "environment key containing =",
                });
            }
            let mut bytes = key.into_vec();
            bytes.push(b'=');
            bytes.extend(value.as_bytes());
            values.push(CString::new(bytes).map_err(|_| PtyTestError::InteriorNul {
                field: "environment",
            })?);
        }
        let mut environment_pointers = values
            .iter()
            .map(|value| value.as_ptr())
            .collect::<Vec<_>>();
        environment_pointers.push(std::ptr::null());
        Ok(Self {
            program,
            _arguments: arguments,
            argument_pointers,
            _environment: values,
            environment_pointers,
            current_dir: command
                .current_dir_path()
                .map(|path| cstring("current directory", path.as_os_str()))
                .transpose()?,
        })
    }
}

fn cstring(field: &'static str, value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| PtyTestError::InteriorNul { field })
}

unsafe fn child_exec(master: RawFd, slave: RawFd, error_fd: RawFd, prepared: &PreparedExec) -> ! {
    if unsafe { libc::close(master) } < 0 {
        unsafe { child_error(error_fd, 1) }
    }
    if unsafe { libc::setsid() } < 0 {
        unsafe { child_error(error_fd, 2) }
    }
    #[cfg(target_os = "linux")]
    let tiocsctty = libc::TIOCSCTTY as libc::Ioctl;
    #[cfg(not(target_os = "linux"))]
    let tiocsctty = libc::TIOCSCTTY as libc::c_ulong;
    if unsafe { libc::ioctl(slave, tiocsctty, 0) } < 0 {
        unsafe { child_error(error_fd, 3) }
    }
    let pid = unsafe { libc::getpid() };
    if unsafe { libc::tcsetpgrp(slave, pid) } < 0 {
        unsafe { child_error(error_fd, 4) }
    }
    if unsafe { libc::tcgetpgrp(slave) } != pid {
        unsafe { child_error_with_errno(error_fd, 5, libc::EPERM) }
    }
    for descriptor in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        if unsafe { libc::dup2(slave, descriptor) } < 0 {
            unsafe { child_error(error_fd, 6) }
        }
    }
    if slave > libc::STDERR_FILENO && unsafe { libc::close(slave) } < 0 {
        unsafe { child_error(error_fd, 7) }
    }
    if let Some(current_dir) = &prepared.current_dir
        && unsafe { libc::chdir(current_dir.as_ptr()) } < 0 {
            unsafe { child_error(error_fd, 8) }
        }
    unsafe {
        libc::execve(
            prepared.program.as_ptr(),
            prepared.argument_pointers.as_ptr(),
            prepared.environment_pointers.as_ptr(),
        );
        child_error(error_fd, 9)
    }
}

unsafe fn child_error(error_fd: RawFd, stage: i32) -> ! {
    unsafe { child_error_with_errno(error_fd, stage, errno()) }
}

unsafe fn child_error_with_errno(error_fd: RawFd, stage: i32, error: i32) -> ! {
    let mut record = [0_u8; 8];
    record[..4].copy_from_slice(&stage.to_ne_bytes());
    record[4..].copy_from_slice(&error.to_ne_bytes());
    let _ = unsafe { libc::write(error_fd, record.as_ptr().cast(), record.len()) };
    unsafe { libc::_exit(127) }
}

fn read_child_error(fd: RawFd) -> Result<Option<(i32, i32)>> {
    let mut record = [0_u8; 8];
    let mut read_total = 0;
    while read_total < record.len() {
        let count = unsafe {
            libc::read(
                fd,
                record[read_total..].as_mut_ptr().cast(),
                record.len() - read_total,
            )
        };
        if count == 0 {
            break;
        }
        if count < 0 && errno() == libc::EINTR {
            continue;
        }
        if count < 0 {
            return Err(system("read spawn error pipe"));
        }
        read_total += count as usize;
    }
    if read_total == 0 {
        return Ok(None);
    }
    if read_total != record.len() {
        return Err(PtyTestError::System {
            operation: "read complete spawn error pipe record",
            errno: libc::EIO,
        });
    }
    let stage = i32::from_ne_bytes(record[..4].try_into().expect("fixed slice length"));
    let error = i32::from_ne_bytes(record[4..].try_into().expect("fixed slice length"));
    Ok(Some((stage, error)))
}

fn reap_rejected_spawn(pid: libc::pid_t) {
    let mut status = 0;
    loop {
        let result = unsafe { libc::waitpid(pid, &mut status, 0) };
        if result == pid || (result < 0 && errno() != libc::EINTR) {
            return;
        }
    }
}

fn stage_name(stage: i32) -> &'static str {
    match stage {
        1 => "close master",
        2 => "setsid",
        3 => "TIOCSCTTY",
        4 => "tcsetpgrp",
        5 => "tcgetpgrp verification",
        6 => "duplicate slave standard descriptor",
        7 => "close original slave",
        8 => "chdir",
        9 => "execve",
        _ => "unknown child setup stage",
    }
}

fn exit_status(status: libc::c_int) -> ExitStatus {
    if libc::WIFEXITED(status) {
        ExitStatus::Code(libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        ExitStatus::Signal(libc::WTERMSIG(status))
    } else {
        ExitStatus::Unreaped
    }
}

pub(crate) fn errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::EIO)
}

fn system(operation: &'static str) -> PtyTestError {
    PtyTestError::System {
        operation,
        errno: errno(),
    }
}
