//! `ptytest` verifies compiled interactive programs at their real Unix
//! terminal boundary.
//!
//! It owns a kernel PTY, drives the program with terminal bytes, and exposes
//! an independent semantic terminal view for assertions. It is deliberately a
//! synchronous, Unix-only test-support crate; it is not a terminal emulator or
//! a general-purpose process library.

#[cfg(not(unix))]
compile_error!("ptytest supports only Unix targets (macOS and Linux)");

mod artifact;
mod config;
mod io;
mod protocol;
mod replay;
mod snapshot;
mod terminal;
mod unix;

pub use config::{CommandSpec, Deadline, Scenario, Size, TestEnv, TestPaths};
pub use io::{ExitStatus, Key, PtyTest};
pub use protocol::ProtocolProfile;
pub use replay::{
    ReplayCheckpoint, ReplayEvent, ReplayOptions, ReplayResult, replay_failure_bundle,
    replay_failure_bundle_with_options,
};
pub use snapshot::{Color, ScreenSnapshot, SnapshotOptions, TerminalBaseline, TerminalState};

use std::fmt;
use std::path::PathBuf;

/// The structured error returned by all crate operations.
#[derive(Debug)]
pub enum PtyTestError {
    /// A terminal dimension was zero.
    InvalidSize { columns: u16, rows: u16 },
    /// A scenario label cannot safely identify an artifact directory.
    InvalidScenarioLabel { label: String, reason: &'static str },
    /// A required spawn setting was not supplied.
    MissingScenarioCommand,
    /// A value contains an interior NUL and cannot cross `execve`.
    InteriorNul { field: &'static str },
    /// A configured UTF-8 locale could not be validated.
    Utf8LocaleUnavailable { locale: String },
    /// Direct execution failed before the application started.
    SpawnFailed { stage: &'static str, errno: i32 },
    /// A Unix system call failed.
    System { operation: &'static str, errno: i32 },
    /// An operation ran past its caller-provided deadline.
    Timeout {
        scenario: String,
        operation: String,
        /// The caller-provided timeout duration when the deadline originated
        /// from [`Deadline::after`]. An absolute deadline intentionally has no
        /// fabricated duration.
        deadline: Option<std::time::Duration>,
        /// Monotonic time since the harness was spawned.
        elapsed: std::time::Duration,
        /// The latest non-consuming status observation of the original child.
        status: ExitStatus,
        /// The semantic terminal dimensions when the timeout was recorded.
        size: Size,
        artifact_dir: Option<PathBuf>,
    },
    /// A terminal query was emitted but the selected profile does not allow it.
    UnsupportedTerminalQuery {
        offset: usize,
        control: String,
        artifact_dir: Option<PathBuf>,
    },
    /// Output violated the selected terminal protocol profile.
    ProtocolViolation {
        offset: usize,
        rule: String,
        control: String,
        artifact_dir: Option<PathBuf>,
    },
    /// The configured raw trace capacity was exhausted.
    TraceLimitExceeded {
        limit: usize,
        artifact_dir: Option<PathBuf>,
    },
    /// A semantic snapshot differs from its stored expectation.
    SnapshotMismatch {
        path: PathBuf,
        diff: String,
        artifact_dir: Option<PathBuf>,
    },
    /// Snapshot updating was not explicitly enabled.
    SnapshotUpdateDisabled { path: PathBuf },
    /// A required fixture or artifact file operation failed.
    Io {
        operation: &'static str,
        path: Option<PathBuf>,
        source: std::io::Error,
    },
    /// The caller requested an input encoding that the selected backend cannot
    /// faithfully derive from terminal state.
    UnsupportedKeyEncoding { key: String },
    /// The supplied signal is not valid for process-group delivery.
    InvalidSignal { signal: i32 },
    /// The child has already been reaped, so group signalling is unsafe.
    ChildAlreadyReaped,
}

impl PtyTestError {
    pub(crate) fn io_at(
        operation: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            operation,
            path: Some(path.into()),
            source,
        }
    }
}

impl fmt::Display for PtyTestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize { columns, rows } => {
                write!(
                    formatter,
                    "terminal size must be non-zero, got {columns}x{rows}"
                )
            }
            Self::InvalidScenarioLabel { label, reason } => {
                write!(formatter, "invalid scenario label {label:?}: {reason}")
            }
            Self::MissingScenarioCommand => formatter.write_str("scenario has no command"),
            Self::InteriorNul { field } => write!(formatter, "{field} contains an interior NUL"),
            Self::Utf8LocaleUnavailable { locale } => {
                write!(
                    formatter,
                    "configured UTF-8 locale is unavailable: {locale}"
                )
            }
            Self::SpawnFailed { stage, errno } => {
                write!(
                    formatter,
                    "PTY child setup failed at {stage}: {}",
                    std::io::Error::from_raw_os_error(*errno)
                )
            }
            Self::System { operation, errno } => {
                write!(
                    formatter,
                    "{operation} failed: {}",
                    std::io::Error::from_raw_os_error(*errno)
                )
            }
            Self::Timeout {
                scenario,
                operation,
                elapsed,
                status,
                size,
                ..
            } => {
                write!(
                    formatter,
                    "scenario {scenario:?} timed out waiting for {operation} after {elapsed:?}; \
                     status={status:?} size={}x{}",
                    size.columns(),
                    size.rows(),
                )
            }
            Self::UnsupportedTerminalQuery {
                offset, control, ..
            } => {
                write!(
                    formatter,
                    "unsupported terminal query at output byte {offset}: {control}"
                )
            }
            Self::ProtocolViolation {
                offset,
                rule,
                control,
                ..
            } => {
                write!(
                    formatter,
                    "protocol rule {rule:?} rejected output byte {offset}: {control}"
                )
            }
            Self::TraceLimitExceeded { limit, .. } => {
                write!(formatter, "raw trace limit of {limit} bytes was exceeded")
            }
            Self::SnapshotMismatch { path, .. } => {
                write!(formatter, "snapshot mismatch: {}", path.display())
            }
            Self::SnapshotUpdateDisabled { path } => write!(
                formatter,
                "snapshot updates are disabled for {}; set PTYTEST_UPDATE_SNAPSHOTS=1 to update explicitly",
                path.display()
            ),
            Self::Io {
                operation,
                path,
                source,
            } => match path {
                Some(path) => write!(formatter, "{operation} {}: {source}", path.display()),
                None => write!(formatter, "{operation}: {source}"),
            },
            Self::UnsupportedKeyEncoding { key } => {
                write!(formatter, "cannot faithfully encode terminal key {key}")
            }
            Self::InvalidSignal { signal } => write!(formatter, "invalid signal: {signal}"),
            Self::ChildAlreadyReaped => formatter.write_str("child has already been reaped"),
        }
    }
}

impl std::error::Error for PtyTestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The result returned by `ptytest` operations.
pub type Result<T> = std::result::Result<T, PtyTestError>;
