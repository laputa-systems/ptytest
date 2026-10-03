use crate::artifact::{ArtifactWriter, FailureContext};
use crate::config::{Deadline, Scenario, ScenarioParts, Size};
use crate::protocol::TerminalPeer;
use crate::snapshot::{ScreenSnapshot, TerminalBaseline, TerminalState};
use crate::terminal::TerminalBackend;
use crate::unix::{self, Spawned};
use crate::{PtyTestError, Result};
use std::fs;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::time::{Duration, Instant};

/// The child status observed through the original child PID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitStatus {
    /// The original child has not yet exited.
    Running,
    /// The child exited normally with this code.
    Code(i32),
    /// The child was terminated by this Unix signal.
    Signal(i32),
    /// The status cannot be observed or has already been reaped.
    Unreaped,
}

/// One ordered, monotonic event in a replayable terminal interaction.
#[derive(Clone, Debug)]
pub(crate) struct TraceEvent {
    pub(crate) sequence: u64,
    pub(crate) elapsed: Duration,
    pub(crate) kind: &'static str,
    pub(crate) payload: String,
}

impl TraceEvent {
    pub(crate) fn summary(&self) -> String {
        format!(
            "#{:04} +{:?} {} {}",
            self.sequence, self.elapsed, self.kind, self.payload
        )
    }
}

/// A mode-aware logical terminal key. `Bytes` is an explicit wire-level
/// override; use [`PtyTest::send_bytes`] for arbitrary byte streams.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    Ctrl(char),
    Bytes(Vec<u8>),
}

/// A real PTY test harness. It is intentionally neither `Clone` nor `Sync`:
/// one test owns one terminal peer, child process group, and event timeline.
pub struct PtyTest {
    scenario: ScenarioParts,
    master: Option<OwnedFd>,
    child_pid: libc::pid_t,
    process_group: libc::pid_t,
    terminal: TerminalBackend,
    peer: TerminalPeer,
    baseline: TerminalBaseline,
    input: Vec<u8>,
    output: Vec<u8>,
    started_at: Instant,
    events: Vec<TraceEvent>,
    artifact: ArtifactWriter,
    observed_exit: ExitStatus,
    reaped: bool,
    closed: bool,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl PtyTest {
    pub fn spawn(scenario: Scenario) -> Result<Self> {
        let scenario = scenario.into_parts()?;
        let Spawned {
            master,
            child_pid,
            process_group,
        } = unix::spawn(&scenario)?;
        let terminal = TerminalBackend::new(scenario.size);
        let baseline = TerminalBaseline(terminal.state()?);
        let peer = TerminalPeer::new(scenario.protocol_profile.clone());
        let started_at = Instant::now();
        let mut result = Self {
            scenario,
            master: Some(master),
            child_pid,
            process_group,
            terminal,
            peer,
            baseline,
            input: Vec::new(),
            output: Vec::new(),
            started_at,
            events: Vec::new(),
            artifact: ArtifactWriter::new(),
            observed_exit: ExitStatus::Running,
            reaped: false,
            closed: false,
            _not_sync: std::marker::PhantomData,
        };
        result.event(
            "spawn",
            format!("pid={child_pid} process-group={process_group}"),
        );
        Ok(result)
    }

    pub fn deadline(&self, duration: Duration) -> Deadline {
        Deadline::after(duration)
    }

    pub fn screen(&self) -> ScreenSnapshot {
        self.terminal
            .snapshot()
            .expect("non-zero terminal backend size")
    }

    pub fn terminal_state(&self) -> TerminalState {
        self.terminal
            .state()
            .expect("non-zero terminal backend size")
    }

    pub fn terminal_baseline(&self) -> TerminalBaseline {
        self.baseline.clone()
    }

    /// Exact bytes written by the test and terminal peer, in wire order.
    pub fn raw_input(&self) -> &[u8] {
        &self.input
    }

    /// Exact bytes read from the PTY master, in wire order.
    pub fn raw_output(&self) -> &[u8] {
        &self.output
    }

    /// Non-blockingly observes the original child's status without reaping it.
    pub fn observe_exit(&mut self) -> Result<Option<ExitStatus>> {
        let status = unix::observe_exit(self.child_pid)?;
        if let Some(status) = status {
            self.observed_exit = status;
            self.event("exit-observed", format!("{status:?}"));
        }
        Ok(status)
    }

    pub fn send_bytes(&mut self, deadline: Deadline, bytes: &[u8]) -> Result<()> {
        self.write_all(deadline, bytes, "write", "")
            .map_err(|error| self.fail("send bytes", error, None))
    }

    pub fn send_text(&mut self, deadline: Deadline, text: &str) -> Result<()> {
        self.send_bytes(deadline, text.as_bytes())
    }

    /// Sends a byte stream in the caller-specified chunks. This exercises the
    /// application's input decoder; it never alters the stream itself.
    pub fn send_fragmented(
        &mut self,
        deadline: Deadline,
        bytes: &[u8],
        chunks: &[usize],
    ) -> Result<()> {
        let mut offset: usize = 0;
        for &length in chunks {
            let end = offset
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .ok_or(PtyTestError::System {
                    operation: "validate input fragments",
                    errno: libc::EINVAL,
                })?;
            self.send_bytes(deadline, &bytes[offset..end])?;
            offset = end;
        }
        if offset != bytes.len() {
            return Err(self.fail(
                "send fragmented",
                PtyTestError::System {
                    operation: "input fragments do not cover byte stream",
                    errno: libc::EINVAL,
                },
                None,
            ));
        }
        Ok(())
    }

    pub fn send_key(&mut self, deadline: Deadline, key: Key) -> Result<()> {
        self.drain(deadline)?;
        let bytes = match key {
            Key::Enter => b"\r".to_vec(),
            Key::Tab => b"\t".to_vec(),
            Key::Backspace => vec![0x7f],
            Key::Escape => vec![0x1b],
            Key::Up => self.cursor_key(b'A')?,
            Key::Down => self.cursor_key(b'B')?,
            Key::Right => self.cursor_key(b'C')?,
            Key::Left => self.cursor_key(b'D')?,
            Key::Home => self.cursor_key(b'H')?,
            Key::End => self.cursor_key(b'F')?,
            Key::Ctrl(character) if character.is_ascii() => {
                vec![(character.to_ascii_uppercase() as u8) & 0x1f]
            }
            Key::Ctrl(character) => {
                return Err(self.fail(
                    "send key",
                    PtyTestError::UnsupportedKeyEncoding {
                        key: format!("Ctrl({character:?})"),
                    },
                    None,
                ));
            }
            Key::Bytes(bytes) => bytes,
        };
        self.send_bytes(deadline, &bytes)
    }

    pub fn send_eof(&mut self, deadline: Deadline) -> Result<()> {
        let master = self
            .master_fd()
            .map_err(|error| self.fail("send EOF", error, None))?;
        let eof = unix::terminal_eof(master).map_err(|error| self.fail("send EOF", error, None))?;
        self.send_bytes(deadline, &[eof])
    }

    /// Closes the PTY master, intentionally simulating terminal disappearance.
    pub fn hangup(&mut self) {
        self.master.take();
        self.closed = true;
        self.event("hangup", String::new());
    }

    pub fn resize(&mut self, size: Size) -> Result<()> {
        let master = self
            .master_fd()
            .map_err(|error| self.fail("resize", error, None))?;
        unix::resize(master, size).map_err(|error| self.fail("resize", error, None))?;
        self.terminal.resize(size);
        self.event(
            "resize",
            format!("columns={} rows={}", size.columns(), size.rows()),
        );
        Ok(())
    }

    pub fn signal(&mut self, signal: i32) -> Result<()> {
        if self.reaped {
            return Err(PtyTestError::ChildAlreadyReaped);
        }
        unix::signal_group(self.process_group, signal)
            .map_err(|error| self.fail("signal process group", error, None))?;
        self.event(
            "signal",
            format!("group={} signal={signal}", self.process_group),
        );
        Ok(())
    }

    /// Drains output which is immediately available. It never uses a sleep as
    /// a synchronization primitive; the deadline only bounds terminal replies.
    pub fn drain(&mut self, deadline: Deadline) -> Result<()> {
        self.read_available(deadline)
            .map_err(|error| self.fail("drain", error, None))
    }

    /// Waits only for a real output event, returning `false` if the supplied
    /// deadline expires. Unlike an assertion wait, an idle terminal is an
    /// ordinary outcome and does not create a failure artifact.
    pub fn wait_for_output(&mut self, deadline: Deadline) -> Result<bool> {
        let initial_length = self.output.len();
        loop {
            self.read_available(deadline)?;
            if self.output.len() != initial_length {
                return Ok(true);
            }
            if deadline.expired() {
                return Ok(false);
            }
            match self.wait_for_activity(deadline) {
                Ok(()) => {}
                Err(PtyTestError::Timeout { .. }) if deadline.expired() => return Ok(false),
                Err(error) => return Err(error),
            }
        }
    }

    /// Waits until no new terminal bytes arrive for `quiet`. This is useful at
    /// a raw-output compatibility boundary where one application action emits
    /// several writes. It is implemented with `poll`, not a thread sleep, and
    /// never extends the caller's overall deadline. `false` means the overall
    /// deadline expired before the terminal became quiet.
    pub fn wait_for_quiescence(&mut self, deadline: Deadline, quiet: Duration) -> Result<bool> {
        if quiet.is_zero() {
            self.read_available(deadline)?;
            return Ok(true);
        }

        loop {
            self.read_available(deadline)?;
            if deadline.expired() {
                return Ok(false);
            }
            let Some(remaining) = deadline.remaining() else {
                return Ok(false);
            };
            let quiet_deadline = Deadline::after(quiet.min(remaining));
            let initial_length = self.output.len();
            match self.wait_for_activity(quiet_deadline) {
                Ok(()) => {}
                Err(PtyTestError::Timeout { .. }) if quiet_deadline.expired() => return Ok(true),
                Err(error) => return Err(error),
            }
            self.read_available(deadline)?;
            if self.output.len() == initial_length {
                return Ok(true);
            }
        }
    }

    pub fn wait_for_screen<F>(
        &mut self,
        deadline: Deadline,
        description: impl Into<String>,
        predicate: F,
    ) -> Result<ScreenSnapshot>
    where
        F: Fn(&ScreenSnapshot) -> bool,
    {
        let description = description.into();
        loop {
            self.read_available(deadline)
                .map_err(|error| self.fail(&description, error, None))?;
            let screen = self.screen();
            let matched = predicate(&screen);
            self.event("predicate", format!("{description:?}={matched}"));
            if matched {
                return Ok(screen);
            }
            if deadline.expired() {
                let error = self.timeout_error(deadline, description);
                return Err(self.fail("wait for screen", error, None));
            }
            self.wait_for_activity(deadline)
                .map_err(|error| self.fail("wait for screen", error, None))?;
        }
    }

    pub fn wait_for_screen_change(
        &mut self,
        deadline: Deadline,
        description: impl Into<String>,
    ) -> Result<ScreenSnapshot> {
        let baseline = self.screen();
        self.wait_for_screen(deadline, description, |screen| screen != &baseline)
    }

    pub fn wait_for_exit(&mut self, deadline: Deadline) -> Result<ExitStatus> {
        loop {
            self.read_available(deadline)
                .map_err(|error| self.fail("wait for exit", error, None))?;
            if let Some(status) = self
                .observe_exit()
                .map_err(|error| self.fail("observe exit", error, None))?
            {
                if let Err(error) = self.peer.finish() {
                    return Err(self.fail("validate terminal output at exit", error, None));
                }
                return Ok(status);
            }
            if deadline.expired() {
                let error = self.timeout_error(deadline, "child exit");
                return Err(self.fail("wait for exit", error, None));
            }
            self.wait_for_activity(deadline)
                .map_err(|error| self.fail("wait for exit", error, None))?;
        }
    }

    pub fn assert_terminal_restored(&mut self, baseline: &TerminalBaseline) -> Result<()> {
        self.drain(self.deadline(Duration::from_millis(1)))?;
        let state = self.terminal_state();
        if state.modes != baseline.state().modes {
            let error = PtyTestError::SnapshotMismatch {
                path: Path::new("terminal-baseline").to_owned(),
                diff: format!(
                    "expected lifecycle modes {:?}, actual {:?}",
                    baseline.state().modes,
                    state.modes
                ),
                artifact_dir: None,
            };
            return Err(self.fail("assert terminal restored", error, None));
        }
        Ok(())
    }

    pub fn assert_snapshot(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let actual = self.screen().to_string();
        match fs::read_to_string(path) {
            Ok(expected) if expected == actual => Ok(()),
            Ok(_expected) if snapshot_updates_enabled() => {
                write_snapshot_atomically(path, &actual)?;
                self.event("snapshot-update", path.display().to_string());
                Ok(())
            }
            Ok(expected) => {
                let diff = inline_diff(&expected, &actual);
                let error = PtyTestError::SnapshotMismatch {
                    path: path.to_owned(),
                    diff: diff.clone(),
                    artifact_dir: None,
                };
                Err(self.fail("assert snapshot", error, Some(&expected)))
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && snapshot_updates_enabled() =>
            {
                write_snapshot_atomically(path, &actual)?;
                self.event("snapshot-update", path.display().to_string());
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let diff = "expected snapshot does not exist".into();
                let failure = PtyTestError::SnapshotMismatch {
                    path: path.to_owned(),
                    diff,
                    artifact_dir: None,
                };
                Err(self.fail("assert snapshot", failure, Some("<missing snapshot>\n")))
            }
            Err(error) => Err(PtyTestError::io_at("read snapshot", path, error)),
        }
    }

    /// Boundedly terminates any remaining original process group and reaps the
    /// original child. It does not assert that the exit code is successful.
    pub fn finish(&mut self, deadline: Deadline) -> Result<ExitStatus> {
        self.cleanup(deadline)
            .map_err(|error| self.fail("finish", error, None))
    }

    fn cursor_key(&self, final_byte: u8) -> Result<Vec<u8>> {
        let state = self.terminal_state();
        if state.modes.application_cursor {
            Ok(vec![0x1b, b'O', final_byte])
        } else {
            Ok(vec![0x1b, b'[', final_byte])
        }
    }

    fn master_fd(&self) -> Result<i32> {
        self.master
            .as_ref()
            .map(AsRawFd::as_raw_fd)
            .ok_or(PtyTestError::System {
                operation: "PTY master is closed",
                errno: libc::EIO,
            })
    }

    fn write_all(
        &mut self,
        deadline: Deadline,
        bytes: &[u8],
        kind: &'static str,
        context: &str,
    ) -> Result<()> {
        let mut offset = 0;
        while offset < bytes.len() {
            let fd = self.master_fd()?;
            let written =
                unsafe { libc::write(fd, bytes[offset..].as_ptr().cast(), bytes.len() - offset) };
            if written > 0 {
                let written = written as usize;
                self.reserve_trace(written)?;
                let input_offset = self.input.len();
                self.input
                    .extend_from_slice(&bytes[offset..offset + written]);
                let range = format!("input-offset={input_offset} input-length={written}");
                let payload = if context.is_empty() {
                    range
                } else {
                    format!("{context} {range}")
                };
                self.event(kind, payload);
                offset += written;
                continue;
            }
            if written < 0 && errno_is_retryable() {
                continue;
            }
            if written < 0 && errno_would_block() {
                self.poll(deadline, libc::POLLOUT)?;
                continue;
            }
            return Err(PtyTestError::System {
                operation: "write PTY master",
                errno: unix::errno(),
            });
        }
        Ok(())
    }

    fn read_available(&mut self, deadline: Deadline) -> Result<()> {
        let Some(master) = self.master.as_ref() else {
            return Ok(());
        };
        let fd = master.as_raw_fd();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if count > 0 {
                let bytes = &buffer[..count as usize];
                self.reserve_trace(bytes.len())?;
                let output_offset = self.output.len();
                self.output.extend_from_slice(bytes);
                self.event(
                    "read",
                    format!(
                        "output-offset={output_offset} output-length={}",
                        bytes.len()
                    ),
                );
                // The model and peer advance together one byte at a time.
                // In particular, CPR must observe the cursor at the query,
                // not the cursor after later bytes in the same kernel read.
                for byte in bytes {
                    let byte = [*byte];
                    self.terminal.process(&byte);
                    let cursor = self.terminal.cursor_position();
                    for reply in self.peer.observe(&byte, cursor)? {
                        self.event(
                            "terminal-query",
                            format!("output-offset={} control={}", reply.offset, reply.control),
                        );
                        let context = format!(
                            "query-output-offset={} control={}",
                            reply.offset, reply.control,
                        );
                        self.write_all(deadline, &reply.bytes, "terminal-reply", &context)?;
                    }
                }
                continue;
            }
            if count == 0 {
                self.closed = true;
                return Ok(());
            }
            if errno_is_retryable() {
                continue;
            }
            if errno_would_block() {
                return Ok(());
            }
            let error = unix::errno();
            if error == libc::EIO {
                self.closed = true;
                return Ok(());
            }
            return Err(PtyTestError::System {
                operation: "read PTY master",
                errno: error,
            });
        }
    }

    fn wait_for_activity(&mut self, deadline: Deadline) -> Result<()> {
        // A finite poll interval lets us observe an exited leader even when a
        // descendant still keeps the slave open. This is kernel polling, not a
        // synchronization sleep, and never extends the caller's deadline.
        self.poll(deadline, libc::POLLIN)
    }

    fn poll(&self, deadline: Deadline, requested_events: i16) -> Result<()> {
        let remaining = deadline
            .remaining()
            .ok_or_else(|| self.timeout_error(deadline, "poll"))?;
        let milliseconds = remaining.as_millis().clamp(1, 25) as libc::c_int;
        let mut descriptor = (!self.closed)
            .then_some(self.master.as_ref())
            .flatten()
            .map(|master| libc::pollfd {
                fd: master.as_raw_fd(),
                events: requested_events,
                revents: 0,
            });
        let (pointer, count) = match descriptor.as_mut() {
            Some(descriptor) => (descriptor as *mut libc::pollfd, 1),
            None => (std::ptr::null_mut(), 0),
        };
        loop {
            let result = unsafe { libc::poll(pointer, count, milliseconds) };
            if result >= 0 {
                return Ok(());
            }
            if unix::errno() == libc::EINTR {
                continue;
            }
            return Err(PtyTestError::System {
                operation: "poll PTY master",
                errno: unix::errno(),
            });
        }
    }

    fn poll_timer(&self, deadline: Deadline) -> Result<()> {
        let remaining = deadline
            .remaining()
            .ok_or_else(|| self.timeout_error(deadline, "cleanup group poll"))?;
        let milliseconds = remaining.as_millis().clamp(1, 25) as libc::c_int;
        loop {
            let result = unsafe { libc::poll(std::ptr::null_mut(), 0, milliseconds) };
            if result >= 0 {
                return Ok(());
            }
            if unix::errno() == libc::EINTR {
                continue;
            }
            return Err(PtyTestError::System {
                operation: "poll cleanup process group",
                errno: unix::errno(),
            });
        }
    }

    fn reserve_trace(&self, added: usize) -> Result<()> {
        if self
            .input
            .len()
            .saturating_add(self.output.len())
            .saturating_add(added)
            > self.scenario.trace_limit
        {
            return Err(PtyTestError::TraceLimitExceeded {
                limit: self.scenario.trace_limit,
                artifact_dir: None,
            });
        }
        Ok(())
    }

    fn cleanup(&mut self, deadline: Deadline) -> Result<ExitStatus> {
        if self.reaped {
            return Ok(self.observed_exit);
        }
        self.read_available(deadline)?;
        let leader_was_observed = if let Some(status) = unix::observe_exit(self.child_pid)? {
            self.observed_exit = status;
            true
        } else {
            false
        };

        // `waitid(WNOWAIT)` leaves the session leader unreaped, so the
        // original process-group ID cannot be reused. It is therefore still
        // safe to signal the entire group even when the leader is a zombie.
        // Reaping first would lose that identity and could leak descendants.
        self.signal_cleanup_group(libc::SIGTERM)?;
        let remaining = deadline
            .remaining()
            .ok_or_else(|| self.timeout_error(deadline, "cleanup child process group"))?;
        let graceful_deadline = Deadline::after(remaining / 2);
        let leader_exited =
            leader_was_observed || self.wait_for_leader_exit(graceful_deadline)?.is_some();
        let group_drained = leader_exited && self.wait_for_group_drain(graceful_deadline)?;
        if !leader_exited || !group_drained {
            self.signal_cleanup_group(libc::SIGKILL)?;
            if !leader_exited && self.wait_for_leader_exit(deadline)?.is_none() {
                return Err(self.timeout_error(deadline, "cleanup child process group"));
            }
            if !self.wait_for_group_drain(deadline)? {
                return Err(self.timeout_error(deadline, "cleanup child process group"));
            }
        }
        self.read_available(deadline)?;
        let status = unix::reap(self.child_pid)?;
        self.observed_exit = status;
        self.reaped = true;
        self.master.take();
        Ok(status)
    }

    fn signal_cleanup_group(&mut self, signal: i32) -> Result<()> {
        match unix::signal_group(self.process_group, signal) {
            Ok(()) => {
                self.event(
                    "cleanup-signal",
                    format!("signal={signal} group={}", self.process_group),
                );
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn wait_for_leader_exit(&mut self, deadline: Deadline) -> Result<Option<ExitStatus>> {
        loop {
            self.read_available(deadline)?;
            if let Some(status) = self.observe_exit()? {
                return Ok(Some(status));
            }
            if deadline.expired() {
                return Ok(None);
            }
            self.wait_for_activity(deadline)?;
        }
    }

    fn wait_for_group_drain(&mut self, deadline: Deadline) -> Result<bool> {
        loop {
            if !unix::group_has_live_member(self.process_group, self.child_pid)? {
                return Ok(true);
            }
            if deadline.expired() {
                return Ok(false);
            }
            // An already closed PTY reports HUP immediately. A descriptor-free
            // poll remains a monotonic kernel wait instead of a busy loop.
            self.poll_timer(deadline)?;
        }
    }

    fn timeout_error(&self, deadline: Deadline, operation: impl Into<String>) -> PtyTestError {
        PtyTestError::Timeout {
            scenario: self.scenario.label.clone(),
            operation: operation.into(),
            deadline: deadline.configured_for(),
            elapsed: deadline.elapsed(),
            status: self.observed_exit,
            size: self.screen().size(),
            artifact_dir: None,
        }
    }

    fn fail(
        &mut self,
        operation: &str,
        error: PtyTestError,
        expected: Option<&str>,
    ) -> PtyTestError {
        let screen = self.screen();
        let state = self.terminal_state();
        match self.artifact.write_failure(FailureContext {
            scenario: &self.scenario,
            operation,
            error: &error,
            input: &self.input,
            output: &self.output,
            screen: &screen,
            terminal_state: &state,
            status: self.observed_exit,
            events: &self.events,
            expected,
        }) {
            Ok(path) => attach_artifact(error, path),
            Err(_) => error,
        }
    }

    fn event(&mut self, kind: &'static str, payload: String) {
        self.events.push(TraceEvent {
            sequence: self.events.len() as u64,
            elapsed: self.started_at.elapsed(),
            kind,
            payload,
        });
    }
}

impl Drop for PtyTest {
    fn drop(&mut self) {
        if !self.reaped {
            let error = PtyTestError::System {
                operation: "live PTY harness dropped",
                errno: libc::ECANCELED,
            };
            let _ = self.fail("drop cleanup", error, None);
            let _ = self.cleanup(Deadline::after(Duration::from_secs(1)));
        }
    }
}

fn errno_is_retryable() -> bool {
    unix::errno() == libc::EINTR
}
fn errno_would_block() -> bool {
    let error = unix::errno();
    error == libc::EAGAIN || error == libc::EWOULDBLOCK
}

fn attach_artifact(error: PtyTestError, path: std::path::PathBuf) -> PtyTestError {
    match error {
        PtyTestError::Timeout {
            scenario,
            operation,
            deadline,
            elapsed,
            status,
            size,
            ..
        } => PtyTestError::Timeout {
            scenario,
            operation,
            deadline,
            elapsed,
            status,
            size,
            artifact_dir: Some(path),
        },
        PtyTestError::UnsupportedTerminalQuery {
            offset, control, ..
        } => PtyTestError::UnsupportedTerminalQuery {
            offset,
            control,
            artifact_dir: Some(path),
        },
        PtyTestError::ProtocolViolation {
            offset,
            rule,
            control,
            ..
        } => PtyTestError::ProtocolViolation {
            offset,
            rule,
            control,
            artifact_dir: Some(path),
        },
        PtyTestError::TraceLimitExceeded { limit, .. } => PtyTestError::TraceLimitExceeded {
            limit,
            artifact_dir: Some(path),
        },
        PtyTestError::SnapshotMismatch {
            path: snapshot_path,
            diff,
            ..
        } => PtyTestError::SnapshotMismatch {
            path: snapshot_path,
            diff,
            artifact_dir: Some(path),
        },
        other => other,
    }
}

fn snapshot_updates_enabled() -> bool {
    std::env::var("PTYTEST_UPDATE_SNAPSHOTS").as_deref() == Ok("1")
}

fn write_snapshot_atomically(path: &Path, contents: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| PtyTestError::io_at("create snapshot directory", parent, error))?;
    let temporary = parent.join(format!(
        ".{}.ptytest-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("snapshot"),
        std::process::id()
    ));
    fs::write(&temporary, contents)
        .map_err(|error| PtyTestError::io_at("write snapshot temporary", &temporary, error))?;
    fs::rename(&temporary, path)
        .map_err(|error| PtyTestError::io_at("replace snapshot", path, error))
}

fn inline_diff(expected: &str, actual: &str) -> String {
    let mut output = String::new();
    for (line, (expected, actual)) in expected.lines().zip(actual.lines()).enumerate() {
        if expected != actual {
            output.push_str(&format!(
                "line {}\n- {}\n+ {}\n",
                line + 1,
                expected,
                actual
            ));
            return output;
        }
    }
    format!(
        "snapshot line count differs: expected {}, actual {}",
        expected.lines().count(),
        actual.lines().count()
    )
}
