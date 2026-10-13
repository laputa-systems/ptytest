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
        && offset >= output.len()
    {
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
            && event.sequence > limit
        {
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
            let record = parse_event_record(line)?;
            if record.version != 1 {
                return Err(invalid_replay("unsupported event trace version"));
            }
            let sequence = record.sequence;
            if sequence != line_number as u128 {
                return Err(invalid_replay("event sequences are not contiguous"));
            }
            let elapsed_ns = record.elapsed_ns;
            if previous_elapsed.is_some_and(|previous| elapsed_ns < previous) {
                return Err(invalid_replay("event times are not monotonic"));
            }
            previous_elapsed = Some(elapsed_ns);
            if !known_event_kind(&record.kind) {
                return Err(invalid_replay("unknown event kind"));
            }
            let sequence = u64::try_from(sequence)
                .map_err(|_| invalid_replay("event sequence exceeds u64"))?;
            Ok(ReplayEvent {
                sequence,
                elapsed_ns,
                kind: record.kind,
                payload: record.payload,
            })
        })
        .collect()
}

struct EventRecord {
    version: u128,
    sequence: u128,
    elapsed_ns: u128,
    kind: String,
    payload: String,
}

fn parse_event_record(line: &str) -> Result<EventRecord> {
    let mut parser = JsonParser::new(line);
    parser.skip_whitespace();
    parser.expect(b'{')?;
    parser.skip_whitespace();

    let mut version = None;
    let mut sequence = None;
    let mut elapsed_ns = None;
    let mut kind = None;
    let mut payload = None;
    if !parser.consume(b'}') {
        loop {
            let key = parser.string()?;
            parser.skip_whitespace();
            parser.expect(b':')?;
            parser.skip_whitespace();
            match key.as_str() {
                "version" => set_once(&mut version, parser.number()?)?,
                "sequence" => set_once(&mut sequence, parser.number()?)?,
                "elapsed_ns" => set_once(&mut elapsed_ns, parser.number()?)?,
                "kind" => set_once(&mut kind, parser.string()?)?,
                "payload" => set_once(&mut payload, parser.string()?)?,
                _ => return Err(invalid_replay("unknown event field")),
            }
            parser.skip_whitespace();
            if parser.consume(b'}') {
                break;
            }
            parser.expect(b',')?;
            parser.skip_whitespace();
        }
    }
    parser.skip_whitespace();
    if !parser.is_finished() {
        return Err(invalid_replay("trailing data after event record"));
    }

    Ok(EventRecord {
        version: version.ok_or_else(|| invalid_replay("event version is absent"))?,
        sequence: sequence.ok_or_else(|| invalid_replay("event sequence is absent"))?,
        elapsed_ns: elapsed_ns.ok_or_else(|| invalid_replay("event elapsed time is absent"))?,
        kind: kind.ok_or_else(|| invalid_replay("event kind is absent"))?,
        payload: payload.ok_or_else(|| invalid_replay("event payload is absent"))?,
    })
}

fn set_once<T>(field: &mut Option<T>, value: T) -> Result<()> {
    if field.replace(value).is_some() {
        return Err(invalid_replay("duplicate event field"));
    }
    Ok(())
}

fn known_event_kind(kind: &str) -> bool {
    matches!(
        kind,
        "spawn"
            | "exit-observed"
            | "hangup"
            | "resize"
            | "signal"
            | "predicate"
            | "snapshot-update"
            | "write"
            | "read"
            | "terminal-query"
            | "terminal-reply"
            | "cleanup-signal"
            | "cleanup-error"
    )
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            offset: 0,
        }
    }

    fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.bytes.get(self.offset), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.offset += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.offset) == Some(&expected) {
            self.offset += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: u8) -> Result<()> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(invalid_replay("malformed event JSON"))
        }
    }

    fn number(&mut self) -> Result<u128> {
        let start = self.offset;
        if self.consume(b'0') {
            if self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                return Err(invalid_replay("invalid event number"));
            }
        } else {
            while self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                self.offset += 1;
            }
            if self.offset == start {
                return Err(invalid_replay("invalid event number"));
            }
        }
        std::str::from_utf8(&self.bytes[start..self.offset])
            .ok()
            .and_then(|number| number.parse().ok())
            .ok_or_else(|| invalid_replay("invalid event number"))
    }

    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut output = String::new();
        loop {
            let byte = *self
                .bytes
                .get(self.offset)
                .ok_or_else(|| invalid_replay("unterminated event string"))?;
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(output);
                }
                b'\\' => {
                    self.offset += 1;
                    let escaped = *self
                        .bytes
                        .get(self.offset)
                        .ok_or_else(|| invalid_replay("unterminated event string escape"))?;
                    self.offset += 1;
                    match escaped {
                        b'"' => output.push('"'),
                        b'\\' => output.push('\\'),
                        b'/' => output.push('/'),
                        b'b' => output.push('\u{0008}'),
                        b'f' => output.push('\u{000c}'),
                        b'n' => output.push('\n'),
                        b'r' => output.push('\r'),
                        b't' => output.push('\t'),
                        b'u' => output.push(self.unicode_escape()?),
                        _ => return Err(invalid_replay("unsupported event string escape")),
                    }
                }
                0x00..=0x1f => return Err(invalid_replay("unescaped control in event string")),
                0x20..=0x7f => {
                    output.push(byte as char);
                    self.offset += 1;
                }
                _ => {
                    let text = std::str::from_utf8(&self.bytes[self.offset..])
                        .map_err(|_| invalid_replay("event string is not UTF-8"))?;
                    let character = text
                        .chars()
                        .next()
                        .ok_or_else(|| invalid_replay("event string is not UTF-8"))?;
                    output.push(character);
                    self.offset += character.len_utf8();
                }
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char> {
        let high = self.hex_quad()?;
        let scalar = match high {
            0xd800..=0xdbff => {
                if !self.consume(b'\\') || !self.consume(b'u') {
                    return Err(invalid_replay("unpaired high surrogate in event string"));
                }
                let low = self.hex_quad()?;
                if !(0xdc00..=0xdfff).contains(&low) {
                    return Err(invalid_replay("invalid low surrogate in event string"));
                }
                0x10000 + (((high as u32 - 0xd800) << 10) | (low as u32 - 0xdc00))
            }
            0xdc00..=0xdfff => {
                return Err(invalid_replay("unpaired low surrogate in event string"));
            }
            value => value as u32,
        };
        char::from_u32(scalar).ok_or_else(|| invalid_replay("invalid unicode event escape"))
    }

    fn hex_quad(&mut self) -> Result<u16> {
        let digits = self
            .bytes
            .get(self.offset..self.offset + 4)
            .ok_or_else(|| invalid_replay("short unicode event escape"))?;
        let digits = std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| u16::from_str_radix(digits, 16).ok())
            .ok_or_else(|| invalid_replay("invalid unicode event escape"))?;
        self.offset += 4;
        Ok(digits)
    }
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

    #[test]
    fn replay_rejects_incomplete_or_unknown_v1_event_records() {
        let valid = event(0, "read", "output-offset=0 output-length=0");
        for malformed in [
            format!("{valid} trailing"),
            valid.replace("\"sequence\":0", "\"sequence\":0,\"extra\":true"),
            valid.replace("\"sequence\":0", "\"sequence\":0,\"version\":1"),
        ] {
            assert!(parse_events(&malformed).is_err(), "accepted {malformed:?}");
        }

        let unknown_kind = event(0, "future-event", "");
        assert!(parse_events(&unknown_kind).is_err());

        let cleanup_error = event(0, "cleanup-error", "process group did not drain");
        assert!(parse_events(&cleanup_error).is_ok());
    }

    #[test]
    fn replay_decodes_valid_json_escapes_in_event_strings() {
        let parsed = parse_events(
            r#"{"version":1,"sequence":0,"elapsed_ns":0,"kind":"predicate","payload":"quoted: \"hello\"\n"}"#,
        )
        .unwrap();
        assert_eq!(parsed[0].payload, "quoted: \"hello\"\n");
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
