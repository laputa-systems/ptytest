use crate::{PtyTestError, Result};

const MAX_CONTROL_BYTES: usize = 1024;

/// An audited terminal output and query-reply contract.
///
/// Profiles are normative: observed output never expands them automatically.
#[derive(Clone, Debug)]
pub struct ProtocolProfile {
    name: &'static str,
    validation_enabled: bool,
    replies_enabled: bool,
}

impl ProtocolProfile {
    pub fn disabled() -> Self {
        Self {
            name: "disabled",
            validation_enabled: false,
            replies_enabled: false,
        }
    }

    /// A deliberately small xterm-compatible contract for normal text,
    /// cursor/editing operations, SGR, and lifecycle modes used by this crate.
    /// OSC, DCS, and mouse extensions remain rejected.
    pub fn xterm_minimal_v1() -> Self {
        Self {
            name: "xterm-minimal-v1",
            validation_enabled: true,
            replies_enabled: true,
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn validation_enabled(&self) -> bool {
        self.validation_enabled
    }
}

#[derive(Debug)]
pub(crate) struct TerminalPeer {
    scanner: ProtocolScanner,
}

/// A deterministic reply to an observed terminal query. Keeping the query
/// offset and control alongside its bytes makes both directions first-class
/// entries in a failure trace.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct TerminalQueryReply {
    pub(crate) offset: usize,
    pub(crate) control: String,
    pub(crate) bytes: Vec<u8>,
}

impl TerminalPeer {
    pub(crate) fn new(profile: ProtocolProfile) -> Self {
        Self {
            scanner: ProtocolScanner::new(profile),
        }
    }

    pub(crate) fn observe(
        &mut self,
        bytes: &[u8],
        cursor: (u16, u16),
    ) -> Result<Vec<TerminalQueryReply>> {
        self.scanner.observe(bytes, cursor)
    }

    pub(crate) fn finish(&mut self) -> Result<()> {
        self.scanner.finish()
    }
}

#[derive(Debug)]
struct ProtocolScanner {
    profile: ProtocolProfile,
    state: ScanState,
    offset: usize,
}

#[derive(Debug)]
enum ScanState {
    Ground,
    Escape { start: usize },
    Csi { start: usize, raw: Vec<u8> },
    Osc { start: usize, raw: Vec<u8> },
    Dcs { start: usize, raw: Vec<u8> },
}

impl ProtocolScanner {
    fn new(profile: ProtocolProfile) -> Self {
        Self {
            profile,
            state: ScanState::Ground,
            offset: 0,
        }
    }

    fn observe(&mut self, bytes: &[u8], cursor: (u16, u16)) -> Result<Vec<TerminalQueryReply>> {
        let mut replies = Vec::new();
        for &byte in bytes {
            let offset = self.offset;
            self.offset += 1;
            if let Some(reply) = self.observe_byte(offset, byte, cursor)? {
                replies.push(reply);
            }
        }
        Ok(replies)
    }

    fn observe_byte(
        &mut self,
        offset: usize,
        byte: u8,
        cursor: (u16, u16),
    ) -> Result<Option<TerminalQueryReply>> {
        let state = std::mem::replace(&mut self.state, ScanState::Ground);
        match state {
            ScanState::Ground => match byte {
                0x1b => {
                    self.state = ScanState::Escape { start: offset };
                    Ok(None)
                }
                0x00..=0x1f | 0x7f => {
                    self.validate_c0(offset, byte)?;
                    Ok(None)
                }
                _ => Ok(None),
            },
            ScanState::Escape { start } => match byte {
                b'[' => {
                    self.state = ScanState::Csi {
                        start,
                        raw: vec![0x1b, b'['],
                    };
                    Ok(None)
                }
                b']' => {
                    self.state = ScanState::Osc {
                        start,
                        raw: vec![0x1b, b']'],
                    };
                    Ok(None)
                }
                b'P' => {
                    self.state = ScanState::Dcs {
                        start,
                        raw: vec![0x1b, b'P'],
                    };
                    Ok(None)
                }
                0x30..=0x7e => self.complete_escape(start, vec![0x1b, byte]),
                0x1b => {
                    self.state = ScanState::Escape { start: offset };
                    self.violation(start, "ESC", "escape sequence was interrupted")
                }
                _ => self.violation(start, "ESC", "invalid escape sequence"),
            },
            ScanState::Csi { start, mut raw } => {
                raw.push(byte);
                if raw.len() > MAX_CONTROL_BYTES {
                    return self.violation(start, "CSI", "control sequence is too long");
                }
                if (0x40..=0x7e).contains(&byte) {
                    self.complete_csi(start, raw, cursor)
                } else if (0x20..=0x3f).contains(&byte) {
                    self.state = ScanState::Csi { start, raw };
                    Ok(None)
                } else {
                    self.violation(start, "CSI", "invalid CSI parameter byte")
                }
            }
            ScanState::Osc { start, mut raw } => {
                raw.push(byte);
                if raw.len() > MAX_CONTROL_BYTES {
                    return self.violation(start, "OSC", "control sequence is too long");
                }
                if byte == 0x07 || raw.ends_with(&[0x1b, b'\\']) {
                    self.complete_string(start, raw, "OSC")
                } else {
                    self.state = ScanState::Osc { start, raw };
                    Ok(None)
                }
            }
            ScanState::Dcs { start, mut raw } => {
                raw.push(byte);
                if raw.len() > MAX_CONTROL_BYTES {
                    return self.violation(start, "DCS", "control sequence is too long");
                }
                if raw.ends_with(&[0x1b, b'\\']) {
                    self.complete_string(start, raw, "DCS")
                } else {
                    self.state = ScanState::Dcs { start, raw };
                    Ok(None)
                }
            }
        }
    }

    fn complete_escape(
        &mut self,
        start: usize,
        raw: Vec<u8>,
    ) -> Result<Option<TerminalQueryReply>> {
        if self.profile.validation_enabled
            && !matches!(
                raw.as_slice(),
                b"\x1b7"
                    | b"\x1b8"
                    | b"\x1bD"
                    | b"\x1bE"
                    | b"\x1bM"
                    | b"\x1bc"
                    | b"\x1b="
                    | b"\x1b>"
            )
        {
            return self.violation(
                start,
                "xterm-minimal-v1/esc",
                &format!("forbidden ESC {}", control_text(&raw)),
            );
        }
        Ok(None)
    }

    fn complete_csi(
        &mut self,
        start: usize,
        raw: Vec<u8>,
        cursor: (u16, u16),
    ) -> Result<Option<TerminalQueryReply>> {
        let final_byte = *raw.last().expect("CSI contains final byte");
        let parameters = &raw[2..raw.len() - 1];
        let query = match (parameters, final_byte) {
            (b"", b'c') => Some(b"\x1b[?62;c".to_vec()),
            (b"5", b'n') => Some(b"\x1b[0n".to_vec()),
            (b"6", b'n') => Some(format!("\x1b[{};{}R", cursor.0 + 1, cursor.1 + 1).into_bytes()),
            _ => None,
        };
        if let Some(reply) = query {
            if !self.profile.replies_enabled {
                return Err(PtyTestError::UnsupportedTerminalQuery {
                    offset: start,
                    control: control_text(&raw),
                    artifact_dir: None,
                });
            }
            return Ok(Some(TerminalQueryReply {
                offset: start,
                control: control_text(&raw),
                bytes: reply,
            }));
        }
        if self.profile.validation_enabled && !allowed_csi(parameters, final_byte) {
            return self.violation(
                start,
                "xterm-minimal-v1/csi",
                &format!("forbidden CSI {}", control_text(&raw)),
            );
        }
        Ok(None)
    }

    fn complete_string(
        &mut self,
        start: usize,
        raw: Vec<u8>,
        family: &'static str,
    ) -> Result<Option<TerminalQueryReply>> {
        if !self.profile.validation_enabled {
            return Ok(None);
        }
        // ish's prompt publishes the current working directory using OSC 7;
        // title metadata (OSC 0/2) is equally harmless to the modeled screen.
        // All other OSC families, including clipboard access, remain denied.
        if family == "OSC"
            && (raw.starts_with(b"\x1b]0;")
                || raw.starts_with(b"\x1b]2;")
                || raw.starts_with(b"\x1b]7;file://"))
        {
            return Ok(None);
        }
        self.violation(
            start,
            "xterm-minimal-v1/string",
            &format!("{family} is not admitted"),
        )
    }

    fn validate_c0(&self, offset: usize, byte: u8) -> Result<()> {
        if self.profile.validation_enabled && !matches!(byte, 0x07 | 0x08 | 0x09 | 0x0a | 0x0d) {
            return Err(PtyTestError::ProtocolViolation {
                offset,
                rule: "xterm-minimal-v1/c0".into(),
                control: format!("C0 0x{byte:02x}"),
                artifact_dir: None,
            });
        }
        Ok(())
    }

    fn violation<T>(&self, offset: usize, rule: &str, control: &str) -> Result<T> {
        Err(PtyTestError::ProtocolViolation {
            offset,
            rule: rule.to_owned(),
            control: control.to_owned(),
            artifact_dir: None,
        })
    }

    fn finish(&mut self) -> Result<()> {
        match &self.state {
            ScanState::Ground => Ok(()),
            ScanState::Escape { start } => self.violation(
                *start,
                "incomplete-control",
                "incomplete ESC at process exit",
            ),
            ScanState::Csi { start, .. } => self.violation(
                *start,
                "incomplete-control",
                "incomplete CSI at process exit",
            ),
            ScanState::Osc { start, .. } => self.violation(
                *start,
                "incomplete-control",
                "incomplete OSC at process exit",
            ),
            ScanState::Dcs { start, .. } => self.violation(
                *start,
                "incomplete-control",
                "incomplete DCS at process exit",
            ),
        }
    }
}

fn allowed_csi(parameters: &[u8], final_byte: u8) -> bool {
    match final_byte {
        b'A' | b'B' | b'C' | b'D' | b'E' | b'F' | b'G' | b'H' | b'f' | b'J' | b'K' | b'L'
        | b'M' | b'P' | b'S' | b'T' | b'X' | b'd' | b'm' | b'r' | b's' => parameters
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b';')),
        // tm turns off the optional kitty keyboard protocol on detach. This
        // is an explicit observed contract, not a blanket admission of CSI
        // intermediates or parameter prefixes.
        b'u' => parameters == b">0",
        b'h' | b'l' => {
            if let Some(parameters) = parameters.strip_prefix(b"?") {
                parameters.split(|byte| *byte == b';').all(|parameter| {
                    matches!(
                        parameter,
                        b"1" | b"25" | b"47" | b"1047" | b"1048" | b"1049" | b"2004" | b"2026"
                        // tm's attached client disables its xterm mouse
                        // tracking before detach. The profile admits only
                        // these DEC mouse selectors, not arbitrary modes.
                        | b"1000" | b"1002" | b"1003" | b"1004" | b"1006"
                    )
                })
            } else {
                parameters
                    .iter()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b';'))
            }
        }
        _ => false,
    }
}

fn control_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| std::ascii::escape_default(*byte))
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xterm_profile_accepts_split_lifecycle_and_sgr_controls() {
        let bytes =
            b"\x1b[?1049h\x1b[?2004h\x1b[?1000l\x1b[?2026h\x1b[>0u\x1b[31mhello\x1b[0m\x1b[?1049l";
        for split in 0..=bytes.len() {
            let mut peer = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
            peer.observe(&bytes[..split], (0, 0)).unwrap();
            peer.observe(&bytes[split..], (0, 0)).unwrap();
            peer.finish().unwrap();
        }
    }

    #[test]
    fn query_replies_are_profile_controlled() {
        let mut peer = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
        assert_eq!(
            peer.observe(b"\x1b[6n", (4, 9)).unwrap(),
            vec![TerminalQueryReply {
                offset: 0,
                control: "\\x1b[6n".into(),
                bytes: b"\x1b[5;10R".to_vec(),
            }],
        );

        let mut disabled = TerminalPeer::new(ProtocolProfile::disabled());
        assert!(matches!(
            disabled.observe(b"\x1b[6n", (0, 0)),
            Err(PtyTestError::UnsupportedTerminalQuery { .. })
        ));

        let mut da = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
        assert_eq!(
            da.observe(b"\x1b[c", (0, 0)).unwrap()[0].bytes,
            b"\x1b[?62;c"
        );
        let mut dsr = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
        assert_eq!(
            dsr.observe(b"\x1b[5n", (0, 0)).unwrap()[0].bytes,
            b"\x1b[0n"
        );
    }

    #[test]
    fn profile_rejects_unreviewed_numeric_kitty_keyboard_controls() {
        for control in [b"\x1b[1u".as_slice(), b"\x1b[>1u", b"\x1b[?0u", b"\x1b[u"] {
            let mut peer = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
            assert!(matches!(
                peer.observe(control, (0, 0)),
                Err(PtyTestError::ProtocolViolation { .. })
            ));
        }
    }

    #[test]
    fn profile_rejects_osc_at_its_start_offset() {
        let mut peer = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
        let error = peer
            .observe(b"text\x1b]52;c;clipboard\x07", (0, 0))
            .unwrap_err();
        assert!(matches!(
            error,
            PtyTestError::ProtocolViolation { offset: 4, .. }
        ));
    }

    #[test]
    fn profile_admits_only_documented_window_title_osc() {
        let mut peer = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
        peer.observe(b"\x1b]0;ish\x07\x1b]2;ish\x1b\\", (0, 0))
            .unwrap();
        let error = peer
            .observe(b"\x1b]52;c;clipboard\x07", (0, 0))
            .unwrap_err();
        assert!(matches!(
            error,
            PtyTestError::ProtocolViolation { offset: 17, .. }
        ));
    }

    #[test]
    fn incomplete_control_is_detected_at_exit() {
        let mut peer = TerminalPeer::new(ProtocolProfile::disabled());
        peer.observe(b"\x1b[?20", (0, 0)).unwrap();
        assert!(matches!(
            peer.finish(),
            Err(PtyTestError::ProtocolViolation { .. })
        ));
    }
}
