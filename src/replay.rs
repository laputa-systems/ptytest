use crate::protocol::{TerminalPeer, TerminalQueryReply};
use crate::terminal::TerminalBackend;
use crate::{ProtocolProfile, PtyTestError, Result, ScreenSnapshot, Size};
use std::collections::VecDeque;
use std::fs;
use std::path::Path;

/// Selects a deterministic inspection point while replaying a failure bundle.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplayOptions {
    event: Option<u64>,
    output_byte: Option<usize>,
    checkpoints: bool,
}

impl ReplayOptions {
    pub fn at_event(mut self, sequence: u64) -> Self {
        self.event = Some(sequence);
        self
    }

    pub fn at_output_byte(mut self, offset: usize) -> Self {
        self.output_byte = Some(offset);
        self
    }

    pub fn with_checkpoints(mut self) -> Self {
        self.checkpoints = true;
        self
    }
}

/// One validated, ordered event from a replay bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayEvent {
    sequence: u64,
    elapsed_ns: u128,
    kind: String,
    payload: String,
}

impl ReplayEvent {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn elapsed_ns(&self) -> u128 {
        self.elapsed_ns
    }
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn payload(&self) -> &str {
        &self.payload
    }
}

/// Semantic state after a recorded read or resize boundary.
#[derive(Clone, Debug)]
pub struct ReplayCheckpoint {
    event_sequence: u64,
    kind: String,
    screen: ScreenSnapshot,
}

impl ReplayCheckpoint {
    pub fn event_sequence(&self) -> u64 {
        self.event_sequence
    }
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn screen(&self) -> &ScreenSnapshot {
        &self.screen
    }
}

/// The semantic state reconstructed from a failure bundle without re-executing
/// the application.
#[derive(Clone, Debug)]
pub struct ReplayResult {
    screen: ScreenSnapshot,
    events_replayed: usize,
    events: Vec<ReplayEvent>,
    checkpoints: Vec<ReplayCheckpoint>,
}

impl ReplayResult {
    pub fn screen(&self) -> &ScreenSnapshot {
        &self.screen
    }
    pub fn events_replayed(&self) -> usize {
        self.events_replayed
    }
    pub fn events(&self) -> &[ReplayEvent] {
        &self.events
    }
    pub fn checkpoints(&self) -> &[ReplayCheckpoint] {
        &self.checkpoints
    }
}

/// Reconstructs the final semantic state from every recorded event boundary.
pub fn replay_failure_bundle(path: impl AsRef<Path>) -> Result<ReplayResult> {
    replay_failure_bundle_with_options(path, ReplayOptions::default())
}

/// Replays a failure bundle through an optional event or byte inspection point.
/// The recorded program is never executed: input, signals, and hangups are
/// validated and exposed as events, while reads and resizes reconstruct state.
pub fn replay_failure_bundle_with_options(
    path: impl AsRef<Path>,
    options: ReplayOptions,
) -> Result<ReplayResult> {
    if options.event.is_some() && options.output_byte.is_some() {
        return Err(invalid_replay(
            "event and output-byte jumps are mutually exclusive",
        ));
    }
    let path = path.as_ref();
    let configuration = read_text(&path.join("configuration.txt"))?;
    let size = replay_size(
        configuration_number(&configuration, "columns=")?,
        configuration_number(&configuration, "rows=")?,
    )?;
    let profile = match configuration_line(&configuration, "profile: ") {
        Some("disabled") => ProtocolProfile::disabled(),
        Some("xterm-minimal-v1") => ProtocolProfile::xterm_minimal_v1(),
        Some(_) => return Err(invalid_replay("unsupported protocol profile")),
        None => return Err(invalid_replay("configuration profile is absent")),
    };
    let input = fs::read(path.join("input.bin"))
        .map_err(|error| PtyTestError::io_at("read replay input", path.join("input.bin"), error))?;
    let output = fs::read(path.join("output.bin")).map_err(|error| {
        PtyTestError::io_at("read replay output", path.join("output.bin"), error)
    })?;
    if let Some(offset) = options.output_byte
        && offset >= output.len() {
            return Err(invalid_replay("output-byte jump exceeds output.bin"));
        }
    let parsed_events = parse_events(&read_text(&path.join("events.jsonl"))?)?;
    let mut terminal = TerminalBackend::new(size);
    let mut peer = TerminalPeer::new(profile);
    let mut replayed = Vec::new();
    let mut checkpoints = Vec::new();
    let mut modeled_events = 0;
    let mut unannounced_replies = VecDeque::new();
    let mut announced_replies = VecDeque::new();
    let mut event_found = options.event.is_none();
    let mut byte_found = options.output_byte.is_none();
    let mut expected_output_offset = 0;
    let mut expected_input_offset = 0;

    for event in parsed_events {
        if let Some(limit) = options.event
            && event.sequence > limit {
                break;
            }
        let mut stop_after_event = false;
        match event.kind.as_str() {
            "resize" => {
                let size = replay_size(
                    payload_number(&event.payload, "columns=")?,
                    payload_number(&event.payload, "rows=")?,
                )?;
                terminal.resize(size);
                modeled_events += 1;
                checkpoint(&mut checkpoints, &terminal, &event, options.checkpoints);
            }
            "read" => {
                let (start, end) = output_range(&event.payload)?;
                if start != expected_output_offset {
                    return Err(invalid_replay("read ranges are not contiguous"));
                }
                let bytes = output
                    .get(start..end)
                    .ok_or_else(|| invalid_replay("read range exceeds output.bin"))?;
                let bytes = if let Some(target) = options.output_byte {
                    if target < start {
                        stop_after_event = true;
                        &bytes[..0]
                    } else if target < end {
                        byte_found = true;
                        stop_after_event = true;
                        &bytes[..=target - start]
                    } else {
                        bytes
                    }
                } else {
                    bytes
                };
                for byte in bytes {
                    let byte = [*byte];
                    terminal.process(&byte);
                    let cursor = terminal.state()?.cursor;
                    unannounced_replies.extend(peer.observe(&byte, (cursor.row, cursor.column))?);
                }
                expected_output_offset = end;
                modeled_events += 1;
                checkpoint(&mut checkpoints, &terminal, &event, options.checkpoints);
            }
            "terminal-query" => {
                validate_query_event(&event, &mut unannounced_replies, &mut announced_replies)?
            }
            "terminal-reply" => {
                let (start, end) = validate_reply_event(&event, &input, &mut announced_replies)?;
                validate_input_range(start, end, input.len(), &mut expected_input_offset)?;
            }
            "write" => {
                let (start, end) = input_range(&event.payload, "input")?;
                validate_input_range(start, end, input.len(), &mut expected_input_offset)?;
            }
            _ => {}
        }
        if options.event == Some(event.sequence) {
            event_found = true;
        }
        replayed.push(event);
        if stop_after_event {
            break;
        }
    }

    if !event_found {
        return Err(invalid_replay("event jump is absent from events.jsonl"));
    }
    if !byte_found {
        return Err(invalid_replay(
            "output-byte jump is absent from read events",
        ));
    }
    if options.event.is_none() && options.output_byte.is_none() {
        if expected_output_offset != output.len() {
            return Err(invalid_replay("read events do not cover output.bin"));
        }
        if expected_input_offset != input.len() {
            return Err(invalid_replay("write events do not cover input.bin"));
        }
        if !unannounced_replies.is_empty() || !announced_replies.is_empty() {
            return Err(invalid_replay("terminal query has no recorded reply"));
        }
        peer.finish()?;
    }
    Ok(ReplayResult {
        screen: terminal.snapshot()?,
        events_replayed: modeled_events,
        events: replayed,
        checkpoints,
    })
}

fn checkpoint(
    checkpoints: &mut Vec<ReplayCheckpoint>,
    terminal: &TerminalBackend,
    event: &ReplayEvent,
    enabled: bool,
) {
    if enabled {
        checkpoints.push(ReplayCheckpoint {
            event_sequence: event.sequence,
            kind: event.kind.clone(),
            screen: terminal
                .snapshot()
                .expect("terminal dimensions stay valid during replay"),
        });
    }
}

fn validate_query_event(
    event: &ReplayEvent,
    unannounced: &mut VecDeque<TerminalQueryReply>,
    announced: &mut VecDeque<TerminalQueryReply>,
) -> Result<()> {
    let reply = unannounced
        .pop_front()
        .ok_or_else(|| invalid_replay("terminal-query has no observed query"))?;
    let offset = payload_number(&event.payload, "output-offset=")?;
    let control = payload_token(&event.payload, "control=")
        .ok_or_else(|| invalid_replay("terminal-query has no control"))?;
    if offset != reply.offset || control != reply.control {
        return Err(invalid_replay(
            "terminal-query does not match replayed output",
        ));
    }
    announced.push_back(reply);
    Ok(())
}

fn validate_reply_event(
    event: &ReplayEvent,
    input: &[u8],
    announced: &mut VecDeque<TerminalQueryReply>,
) -> Result<(usize, usize)> {
    let reply = announced
        .pop_front()
        .ok_or_else(|| invalid_replay("terminal-reply has no observed query"))?;
    let query_offset = payload_number(&event.payload, "query-output-offset=")?;
    let control = payload_token(&event.payload, "control=")
        .ok_or_else(|| invalid_replay("terminal-reply has no control"))?;
    let (start, end) = input_range(&event.payload, "input")?;
    if query_offset != reply.offset
        || control != reply.control
        || input.get(start..end) != Some(reply.bytes.as_slice())
    {
        return Err(invalid_replay(
            "terminal-reply does not match generated response",
        ));
    }
    Ok((start, end))
}

fn validate_input_range(
    start: usize,
    end: usize,
    length: usize,
    expected: &mut usize,
) -> Result<()> {
    if start != *expected {
        return Err(invalid_replay("input ranges are not contiguous"));
    }
    if end > length {
        return Err(invalid_replay("input range exceeds input.bin"));
    }
    *expected = end;
    Ok(())
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|error| PtyTestError::io_at("read replay artifact", path, error))
}

fn parse_events(events: &str) -> Result<Vec<ReplayEvent>> {
    let mut previous_elapsed = None;
    events
        .lines()
        .enumerate()
        .map(|(line_number, line)| {
            let version = json_number(line, "version")
                .ok_or_else(|| invalid_replay("event version is absent"))?;
            if version != 1 {
                return Err(invalid_replay("unsupported event trace version"));
            }
            let sequence = json_number(line, "sequence")
                .ok_or_else(|| invalid_replay("event sequence is absent"))?;
            if sequence != line_number as u128 {
                return Err(invalid_replay("event sequences are not contiguous"));
            }
            let elapsed_ns = json_number(line, "elapsed_ns")
                .ok_or_else(|| invalid_replay("event elapsed time is absent"))?;
            if previous_elapsed.is_some_and(|previous| elapsed_ns < previous) {
                return Err(invalid_replay("event times are not monotonic"));
            }
            previous_elapsed = Some(elapsed_ns);
            let kind = json_string(line, "kind")?;
            let payload = json_string(line, "payload")?;
            let sequence = u64::try_from(sequence)
                .map_err(|_| invalid_replay("event sequence exceeds u64"))?;
            Ok(ReplayEvent {
                sequence,
                elapsed_ns,
                kind,
                payload,
            })
        })
        .collect()
}

fn configuration_line<'a>(configuration: &'a str, prefix: &str) -> Option<&'a str> {
    configuration
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
}

fn configuration_number(configuration: &str, prefix: &str) -> Result<usize> {
    let start = configuration
        .find(prefix)
        .ok_or_else(|| invalid_replay("configuration field is absent"))?
        + prefix.len();
    configuration[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .map_err(|_| invalid_replay("configuration number is invalid"))
}

fn payload_number(payload: &str, prefix: &str) -> Result<usize> {
    let start = payload
        .find(prefix)
        .ok_or_else(|| invalid_replay("event field is absent"))?
        + prefix.len();
    payload[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .map_err(|_| invalid_replay("event number is invalid"))
}

fn payload_text<'a>(payload: &'a str, prefix: &str) -> Option<&'a str> {
    payload
        .find(prefix)
        .map(|start| &payload[start + prefix.len()..])
}

fn payload_token<'a>(payload: &'a str, prefix: &str) -> Option<&'a str> {
    payload_text(payload, prefix)?
        .split_ascii_whitespace()
        .next()
}

fn output_range(payload: &str) -> Result<(usize, usize)> {
    if let Some(range) = payload
        .strip_prefix("output=[")
        .and_then(|value| value.strip_suffix(']'))
    {
        let (start, end) = range
            .split_once(',')
            .ok_or_else(|| invalid_replay("invalid legacy output range"))?;
        return Ok((
            start
                .parse()
                .map_err(|_| invalid_replay("invalid legacy output range start"))?,
            end.parse()
                .map_err(|_| invalid_replay("invalid legacy output range end"))?,
        ));
    }
    input_range(payload, "output")
}

fn replay_size(columns: usize, rows: usize) -> Result<Size> {
    let columns =
        u16::try_from(columns).map_err(|_| invalid_replay("terminal columns exceed u16"))?;
    let rows = u16::try_from(rows).map_err(|_| invalid_replay("terminal rows exceed u16"))?;
    Size::new(columns, rows)
}

fn input_range(payload: &str, name: &str) -> Result<(usize, usize)> {
    let offset = payload_number(payload, &format!("{name}-offset="))?;
    let length = payload_number(payload, &format!("{name}-length="))?;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| invalid_replay("event range overflows"))?;
    Ok((offset, end))
}

fn json_number(line: &str, name: &str) -> Option<u128> {
    let prefix = format!("\"{name}\":");
    let value = line.split_once(&prefix)?.1;
    let digits = value
        .bytes()
        .take_while(u8::is_ascii_digit)
        .collect::<Vec<_>>();
    (!digits.is_empty()).then(|| std::str::from_utf8(&digits).ok()?.parse().ok())?
}

fn json_string(line: &str, name: &str) -> Result<String> {
    let prefix = format!("\"{name}\":\"");
    let bytes = line
        .split_once(&prefix)
        .ok_or_else(|| invalid_replay("event string field is absent"))?
        .1
        .as_bytes();
    let mut decoded = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                return String::from_utf8(decoded)
                    .map_err(|_| invalid_replay("event string is not UTF-8"));
            }
            b'\\' => {
                index += 1;
                let escaped = *bytes
                    .get(index)
                    .ok_or_else(|| invalid_replay("unterminated event string escape"))?;
                match escaped {
                    b'"' => decoded.push(b'"'),
                    b'\\' => decoded.push(b'\\'),
                    b'n' => decoded.push(b'\n'),
                    b'r' => decoded.push(b'\r'),
                    b't' => decoded.push(b'\t'),
                    b'u' => {
                        let hex = bytes
                            .get(index + 1..index + 5)
                            .ok_or_else(|| invalid_replay("short unicode event escape"))?;
                        let value = std::str::from_utf8(hex)
                            .ok()
                            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                            .and_then(char::from_u32)
                            .ok_or_else(|| invalid_replay("invalid unicode event escape"))?;
                        let mut encoded = [0; 4];
                        decoded.extend_from_slice(value.encode_utf8(&mut encoded).as_bytes());
                        index += 4;
                    }
                    _ => return Err(invalid_replay("unsupported event string escape")),
                }
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    Err(invalid_replay("unterminated event string"))
}

fn invalid_replay(detail: &'static str) -> PtyTestError {
    PtyTestError::Io {
        operation: detail,
        path: None,
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static BUNDLE_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn output_ranges_are_strict() {
        assert_eq!(
            output_range("output-offset=2 output-length=4").unwrap(),
            (2, 6)
        );
        assert!(output_range("output-offset=2 output-length=x").is_err());
    }

    #[test]
    fn replay_validates_query_replies_and_supports_event_and_byte_jumps() {
        let root = test_bundle(
            b"\x1b[1;2R",
            b"A\x1b[6nBC",
            &[
                event(0, "read", "output-offset=0 output-length=7"),
                event(1, "terminal-query", "output-offset=1 control=\\x1b[6n"),
                event(
                    2,
                    "terminal-reply",
                    "query-output-offset=1 control=\\x1b[6n input-offset=0 input-length=6",
                ),
                event(3, "resize", "columns=5 rows=2"),
            ],
        );
        let result =
            replay_failure_bundle_with_options(&root, ReplayOptions::default().with_checkpoints())
                .unwrap();
        assert!(result.screen().contains("ABC"));
        assert_eq!(result.events_replayed(), 2);
        assert_eq!(result.checkpoints().len(), 2);
        assert_eq!(result.events().len(), 4);

        let by_event =
            replay_failure_bundle_with_options(&root, ReplayOptions::default().at_event(0))
                .unwrap();
        assert!(by_event.screen().contains("ABC"));
        assert_eq!(by_event.events().len(), 1);

        let by_byte =
            replay_failure_bundle_with_options(&root, ReplayOptions::default().at_output_byte(0))
                .unwrap();
        assert!(by_byte.screen().contains("A"));
        assert!(!by_byte.screen().contains("B"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replay_rejects_bad_event_schema_and_reply_bytes() {
        let root = test_bundle(
            b"wrong",
            b"A\x1b[6n",
            &[event(0, "read", "output-offset=0 output-length=5")],
        );
        assert!(replay_failure_bundle(&root).is_err());
        fs::remove_dir_all(root).unwrap();

        assert!(parse_events("{\"version\":2,\"sequence\":0,\"elapsed_ns\":0,\"kind\":\"read\",\"payload\":\"output-offset=0 output-length=0\"}\n").is_err());
        assert!(parse_events("{\"version\":1,\"sequence\":1,\"elapsed_ns\":0,\"kind\":\"read\",\"payload\":\"output-offset=0 output-length=0\"}\n").is_err());
    }

    fn event(sequence: u64, kind: &str, payload: &str) -> String {
        let payload = payload.replace('\\', "\\\\");
        format!(
            "{{\"version\":1,\"sequence\":{sequence},\"elapsed_ns\":0,\"kind\":\"{kind}\",\"payload\":\"{payload}\"}}"
        )
    }

    fn test_bundle(input: &[u8], output: &[u8], events: &[String]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "ptytest-replay-test-{}-{}",
            std::process::id(),
            BUNDLE_COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("configuration.txt"),
            "size: columns=4 rows=2\nprofile: xterm-minimal-v1\n",
        )
        .unwrap();
        fs::write(root.join("input.bin"), input).unwrap();
        fs::write(root.join("output.bin"), output).unwrap();
        fs::write(
            root.join("events.jsonl"),
            format!("{}\n", events.join("\n")),
        )
        .unwrap();
        root
    }
}
