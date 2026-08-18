//! Failure-bundle serialization lives here so lifecycle code never silently
//! drops the evidence that explains a timeout or assertion failure.

use crate::config::ScenarioParts;
use crate::io::TraceEvent;
use crate::snapshot::{ScreenSnapshot, TerminalState};
use crate::{ExitStatus, PtyTestError, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static ARTIFACT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(crate) struct ArtifactWriter {
    root: Option<PathBuf>,
}

impl ArtifactWriter {
    pub(crate) fn new() -> Self {
        Self { root: None }
    }

    pub(crate) fn write_failure(
        &mut self,
        scenario: &ScenarioParts,
        operation: &str,
        error: &PtyTestError,
        input: &[u8],
        output: &[u8],
        screen: &ScreenSnapshot,
        terminal_state: &TerminalState,
        status: ExitStatus,
        events: &[TraceEvent],
        expected: Option<&str>,
    ) -> Result<PathBuf> {
        let root = match &self.root {
            Some(root) => root.clone(),
            None => {
                let target = PathBuf::from("target/ptytest-failures");
                fs::create_dir_all(&target).map_err(|error| {
                    PtyTestError::io_at("create artifact parent", &target, error)
                })?;
                let counter = ARTIFACT_COUNTER.fetch_add(1, Ordering::Relaxed);
                let root = target.join(format!(
                    "{}-{}-{counter}",
                    scenario.artifact_label,
                    std::process::id()
                ));
                fs::create_dir(&root).map_err(|error| {
                    PtyTestError::io_at("create artifact directory", &root, error)
                })?;
                self.root = Some(root.clone());
                root
            }
        };
        write(&root.join("input.bin"), input)?;
        write(&root.join("output.bin"), output)?;
        write(&root.join("screen.ptytest"), screen.to_string().as_bytes())?;
        write(
            &root.join("terminal-state.txt"),
            format!("{terminal_state:#?}\n").as_bytes(),
        )?;
        write(
            &root.join("exit-status.txt"),
            format!("{status:?}\n").as_bytes(),
        )?;
        let elapsed = events.last().map(|event| event.elapsed).unwrap_or_default();
        let timeout = match error {
            PtyTestError::Timeout { deadline, .. } => deadline
                .map(|duration| duration.as_nanos().to_string())
                .unwrap_or_else(|| "absolute".into()),
            _ => "not-applicable".into(),
        };
        write(&root.join("configuration.txt"), format!(
            "scenario: {}\nsize: columns={} rows={}\nprofile: {}\ntrace-limit: {}\nplatform: {}\ninput-bytes: {}\noutput-bytes: {}\nevent-count: {}\nelapsed-ns: {}\ntimeout-deadline-ns: {timeout}\n",
            scenario.label,
            scenario.size.columns(),
            scenario.size.rows(),
            scenario.protocol_profile.name(),
            scenario.trace_limit,
            std::env::consts::OS,
            input.len(),
            output.len(),
            events.len(),
            elapsed.as_nanos(),
        ).as_bytes())?;
        write(
            &root.join("command.txt"),
            scenario.command_description().as_bytes(),
        )?;
        write(
            &root.join("events.log"),
            events
                .iter()
                .map(TraceEvent::summary)
                .collect::<Vec<_>>()
                .join("\n")
                .as_bytes(),
        )?;
        write(
            &root.join("events.jsonl"),
            events.iter().map(event_json).collect::<String>().as_bytes(),
        )?;
        write(
            &root.join("failure.txt"),
            format!("operation: {operation}\nerror: {error}\n").as_bytes(),
        )?;
        if let Some(expected) = expected {
            write(&root.join("expected.ptytest"), expected.as_bytes())?;
            let actual = screen.to_string();
            write(&root.join("actual.ptytest"), actual.as_bytes())?;
            write(
                &root.join("diff.txt"),
                concise_diff(expected, &actual).as_bytes(),
            )?;
        }
        Ok(root)
    }
}

fn concise_diff(expected: &str, actual: &str) -> String {
    for (index, (expected, actual)) in expected.lines().zip(actual.lines()).enumerate() {
        if expected != actual {
            return format!("line {}\n- {}\n+ {}\n", index + 1, expected, actual);
        }
    }
    format!(
        "line count differs: expected {}, actual {}\n",
        expected.lines().count(),
        actual.lines().count()
    )
}

fn event_json(event: &TraceEvent) -> String {
    format!(
        "{{\"version\":1,\"sequence\":{},\"elapsed_ns\":{},\"kind\":{},\"payload\":{}}}\n",
        event.sequence,
        event.elapsed.as_nanos(),
        json_string(event.kind),
        json_string(&event.payload),
    )
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|error| PtyTestError::io_at("write artifact", path, error))
}
