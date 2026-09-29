use crate::snapshot::{
    CellAttributes, Color, Cursor, MouseMode, ScreenSnapshot, TerminalModes, TerminalState,
    cell_snapshot,
};
use crate::{Result, Size};

/// Private boundary around the independent `vt100` terminal model.
pub(crate) struct TerminalBackend {
    parser: vt100::Parser,
}

impl TerminalBackend {
    pub(crate) fn new(size: Size) -> Self {
        Self {
            parser: vt100::Parser::new(size.rows(), size.columns(), 0),
        }
    }

    pub(crate) fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub(crate) fn resize(&mut self, size: Size) {
        self.parser
            .screen_mut()
            .set_size(size.rows(), size.columns());
    }

    pub(crate) fn snapshot(&self) -> Result<ScreenSnapshot> {
        let screen = self.parser.screen();
        let (rows, columns) = screen.size();
        let size = Size::new(columns, rows)?;
        let (cursor_row, cursor_column) = screen.cursor_position();
        let modes = TerminalModes {
            alternate_screen: screen.alternate_screen(),
            application_cursor: screen.application_cursor(),
            application_keypad: screen.application_keypad(),
            bracketed_paste: screen.bracketed_paste(),
            cursor_visible: !screen.hide_cursor(),
            mouse: match screen.mouse_protocol_mode() {
                vt100::MouseProtocolMode::None => MouseMode::Disabled,
                vt100::MouseProtocolMode::Press => MouseMode::Press,
                vt100::MouseProtocolMode::PressRelease => MouseMode::PressRelease,
                vt100::MouseProtocolMode::ButtonMotion => MouseMode::ButtonMotion,
                vt100::MouseProtocolMode::AnyMotion => MouseMode::AnyMotion,
            },
        };
        let screen_rows = (0..rows)
            .map(|row| {
                (0..columns)
                    .map(|column| {
                        let cell = screen
                            .cell(row, column)
                            .expect("vt100 has every visible cell");
                        let attributes = CellAttributes {
                            foreground: map_color(cell.fgcolor()),
                            background: map_color(cell.bgcolor()),
                            bold: cell.bold(),
                            dim: cell.dim(),
                            italic: cell.italic(),
                            underline: cell.underline(),
                            inverse: cell.inverse(),
                        };
                        cell_snapshot(
                            cell.contents().to_owned(),
                            cell.is_wide(),
                            cell.is_wide_continuation(),
                            attributes,
                        )
                    })
                    .collect()
            })
            .collect();
        Ok(ScreenSnapshot::new(
            size,
            Cursor {
                row: cursor_row,
                column: cursor_column,
                visible: !screen.hide_cursor(),
            },
            modes,
            screen_rows,
            (0..rows).map(|row| screen.row_wrapped(row)).collect(),
        ))
    }

    pub(crate) fn state(&self) -> Result<TerminalState> {
        let snapshot = self.snapshot()?;
        Ok(TerminalState::from_snapshot(
            &snapshot,
            self.parser.screen().scrollback(),
        ))
    }
}

fn map_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Default,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProtocolProfile;
    use crate::protocol::TerminalPeer;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::{Column as AlacrittyColumn, Line as AlacrittyLine};
    use alacritty_terminal::term::cell::{Cell as AlacrittyCell, Flags};
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{Config as AlacrittyConfig, Term as AlacrittyTerm, TermMode};
    use alacritty_terminal::vte::ansi::{Color as AlacrittyColor, NamedColor};
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn terminal_state_is_independent_of_output_chunk_boundaries() {
        let size = Size::new(8, 2).unwrap();
        let bytes = b"\x1b[?1049h\xe7\x95\x8c\x1b[?2004h";
        let mut one_chunk = TerminalBackend::new(size);
        one_chunk.process(bytes);
        let expected = one_chunk.snapshot().unwrap();
        for split in 0..=bytes.len() {
            let mut split_chunks = TerminalBackend::new(size);
            split_chunks.process(&bytes[..split]);
            split_chunks.process(&bytes[split..]);
            assert_eq!(
                split_chunks.snapshot().unwrap(),
                expected,
                "split at {split}"
            );
        }
    }

    #[test]
    fn semantic_snapshot_survives_every_partition_of_split_controls_and_utf8() {
        let size = Size::new(8, 2).unwrap();
        // Thirteen boundaries keeps this exhaustive while crossing CSI and
        // multi-byte UTF-8/combining-byte boundaries independently.
        let bytes = b"\x1b[31m\xe7\x95\x8c\xcc\x81\x1b[0m";
        let mut contiguous = TerminalBackend::new(size);
        contiguous.process(bytes);
        let expected = contiguous.snapshot().unwrap();

        for partition in 0..(1_u16 << (bytes.len() - 1)) {
            let mut terminal = TerminalBackend::new(size);
            let mut start = 0;
            for boundary in 0..bytes.len() - 1 {
                if partition & (1 << boundary) != 0 {
                    terminal.process(&bytes[start..=boundary]);
                    start = boundary + 1;
                }
            }
            terminal.process(&bytes[start..]);
            assert_eq!(
                terminal.snapshot().unwrap(),
                expected,
                "partition {partition:013b}"
            );
        }
    }

    #[test]
    fn snapshot_exposes_visible_attributes_without_vt100_types() {
        let mut terminal = TerminalBackend::new(Size::new(4, 1).unwrap());
        terminal.process(b"\x1b[1;31mX\x1b[0m");
        let snapshot = terminal.snapshot().unwrap();
        let cell = snapshot.cell(0, 0).unwrap();
        assert!(cell.attributes().bold);
        assert_eq!(
            cell.attributes().foreground,
            crate::snapshot::Color::Indexed(1)
        );
        assert!(
            snapshot
                .to_text(crate::SnapshotOptions::default().with_attributes())
                .contains("columns=0..1")
        );
    }

    // These semantic regressions used to be duplicated in the e and ish
    // private harnesses. They belong here, where every consumer receives the
    // same proof without retaining a public `vt100` parser dependency.
    #[test]
    fn shared_backend_retains_split_clear_home_and_unicode_erase_semantics() {
        let mut split_control = TerminalBackend::new(Size::new(12, 3).unwrap());
        split_control.process(b"before\x1b[");
        split_control.process(b"2J\x1b[Hafter");
        let snapshot = split_control.snapshot().unwrap();
        assert_eq!(snapshot.row(0).as_deref(), Some("after"));

        let mut unicode = TerminalBackend::new(Size::new(12, 3).unwrap());
        unicode.process("日本語".as_bytes());
        assert_eq!(unicode.snapshot().unwrap().cursor().column, 6);
        unicode.process(b"\x1b[H\x1b[31mred\x1b[0m\x1b[K");
        let snapshot = unicode.snapshot().unwrap();
        assert_eq!(snapshot.cell(0, 0).unwrap().contents(), "r");
        assert_eq!(
            snapshot.cell(0, 0).unwrap().attributes().foreground,
            crate::snapshot::Color::Indexed(1)
        );
        assert!(snapshot.cell(0, 5).unwrap().contents().is_empty());
    }

    // This is deliberately a capability-normalized comparison rather than a
    // backend-internal one. The stable contract is the visible viewport,
    // cursor, basic attributes, wrapping, and alternate-screen lifecycle.
    // Backend-specific features such as scrollback implementation details,
    // hyperlink storage, and palette resolution are intentionally absent.
    //
    // The recordings are checked in under `tests/fixtures/differential/` and
    // are replayed in their recorded chunks. A disagreement writes both
    // normalized states, versions, and the exact offending event to
    // `target/ptytest-differential/` before failing.
    #[test]
    fn differential_oracle_matches_canonical_terminal_corpus() {
        for trace in CANONICAL_TRACES {
            run_differential_trace(trace);
        }
    }

    const CANONICAL_TRACES: &[CanonicalTrace] = &[
        CanonicalTrace {
            name: "startup-and-cursor",
            columns: 12,
            rows: 3,
            recording: include_str!("../tests/fixtures/differential/startup-and-cursor.trace"),
        },
        CanonicalTrace {
            name: "alternate-lifecycle",
            columns: 12,
            rows: 3,
            recording: include_str!("../tests/fixtures/differential/alternate-lifecycle.trace"),
        },
        CanonicalTrace {
            name: "unicode-and-erasure",
            columns: 8,
            rows: 2,
            recording: include_str!("../tests/fixtures/differential/unicode-and-erasure.trace"),
        },
        CanonicalTrace {
            name: "resize-explicit-boundaries",
            columns: 6,
            rows: 3,
            recording: include_str!(
                "../tests/fixtures/differential/resize-explicit-boundaries.trace"
            ),
        },
        CanonicalTrace {
            name: "mode-lifecycle",
            columns: 12,
            rows: 2,
            recording: include_str!("../tests/fixtures/differential/mode-lifecycle.trace"),
        },
        CanonicalTrace {
            name: "tm2-detach",
            columns: 12,
            rows: 2,
            recording: include_str!("../tests/fixtures/differential/tm2-detach.trace"),
        },
        CanonicalTrace {
            name: "e-synchronized-redraw",
            columns: 12,
            rows: 2,
            recording: include_str!("../tests/fixtures/differential/e-synchronized-redraw.trace"),
        },
        CanonicalTrace {
            name: "ish-working-directory",
            columns: 12,
            rows: 2,
            recording: include_str!("../tests/fixtures/differential/ish-working-directory.trace"),
        },
    ];

    struct CanonicalTrace {
        name: &'static str,
        columns: u16,
        rows: u16,
        recording: &'static str,
    }

    #[derive(Debug)]
    enum DifferentialOperation {
        Bytes {
            source_line: usize,
            bytes: Vec<u8>,
        },
        Resize {
            source_line: usize,
            columns: u16,
            rows: u16,
        },
    }

    #[derive(Debug, Eq, PartialEq)]
    struct NormalizedSnapshot {
        columns: u16,
        rows: u16,
        cursor: crate::snapshot::Cursor,
        modes: NormalizedModes,
        viewport: Vec<NormalizedRow>,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct NormalizedModes {
        alternate_screen: bool,
        application_cursor: bool,
        application_keypad: bool,
        bracketed_paste: bool,
        cursor_visible: bool,
        mouse_enabled: bool,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct NormalizedRow {
        cells: Vec<NormalizedCell>,
        wrapped: bool,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct NormalizedCell {
        contents: String,
        wide: bool,
        wide_continuation: bool,
        attributes: crate::snapshot::CellAttributes,
    }

    fn run_differential_trace(trace: &CanonicalTrace) {
        let initial_size =
            Size::new(trace.columns, trace.rows).expect("canonical trace has a valid size");
        let mut primary = TerminalBackend::new(initial_size);
        let mut reference = AlacrittyTerm::new(
            AlacrittyConfig {
                scrolling_history: 0,
                ..Default::default()
            },
            &TermSize::new(usize::from(trace.columns), usize::from(trace.rows)),
            VoidListener,
        );
        let mut reference_parser: alacritty_terminal::vte::ansi::Processor =
            alacritty_terminal::vte::ansi::Processor::new();
        let mut current_size = initial_size;
        let mut event_log = Vec::new();

        for (operation_index, operation) in parse_trace(trace).into_iter().enumerate() {
            let description = match &operation {
                DifferentialOperation::Bytes { source_line, bytes } => {
                    primary.process(bytes);
                    reference_parser.advance(&mut reference, bytes);
                    format!("trace line {source_line}: bytes {}", hex(bytes))
                }
                DifferentialOperation::Resize {
                    source_line,
                    columns,
                    rows,
                } => {
                    current_size =
                        Size::new(*columns, *rows).expect("canonical trace has a valid resize");
                    primary.resize(current_size);
                    reference.resize(TermSize::new(usize::from(*columns), usize::from(*rows)));
                    format!("trace line {source_line}: resize columns={columns} rows={rows}")
                }
            };
            event_log.push(description.clone());

            let primary_state = normalized_primary(&primary);
            let reference_state = normalized_alacritty(&reference, current_size);
            if primary_state != reference_state {
                let artifact_dir = preserve_differential_failure(
                    trace,
                    operation_index,
                    &description,
                    &event_log,
                    &primary_state,
                    &reference_state,
                );
                panic!(
                    "terminal-model disagreement in {} after operation {operation_index}: {description}\n\
                     primary=vt100 0.16.2 reference=alacritty_terminal 0.26.0\n\
                     artifact={}\nprimary state:\n{primary_state:#?}\nreference state:\n{reference_state:#?}",
                    trace.name,
                    artifact_dir.display(),
                );
            }
        }
    }

    fn parse_trace(trace: &CanonicalTrace) -> Vec<DifferentialOperation> {
        trace
            .recording
            .lines()
            .enumerate()
            .filter_map(|(index, raw_line)| {
                let source_line = index + 1;
                let line = raw_line.trim();
                if line.is_empty() || line.starts_with('#') {
                    return None;
                }
                if let Some(encoded) = line.strip_prefix("bytes: ") {
                    return Some(DifferentialOperation::Bytes {
                        source_line,
                        bytes: decode_hex(trace.name, source_line, encoded),
                    });
                }
                if let Some(dimensions) = line.strip_prefix("resize: ") {
                    let mut values = dimensions.split_ascii_whitespace();
                    let columns = values
                        .next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or_else(|| {
                            panic!("{}:{source_line}: resize needs a column count", trace.name)
                        });
                    let rows = values
                        .next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or_else(|| {
                            panic!("{}:{source_line}: resize needs a row count", trace.name)
                        });
                    assert!(
                        values.next().is_none(),
                        "{}:{source_line}: resize accepts exactly two values",
                        trace.name
                    );
                    return Some(DifferentialOperation::Resize {
                        source_line,
                        columns,
                        rows,
                    });
                }
                panic!(
                    "{}:{source_line}: unrecognized canonical trace event {line:?}",
                    trace.name
                );
            })
            .collect()
    }

    fn decode_hex(trace_name: &str, source_line: usize, encoded: &str) -> Vec<u8> {
        assert!(
            encoded.len().is_multiple_of(2),
            "{trace_name}:{source_line}: byte event has odd-length hex"
        );
        (0..encoded.len())
            .step_by(2)
            .map(|offset| {
                u8::from_str_radix(&encoded[offset..offset + 2], 16).unwrap_or_else(|_| {
                    panic!("{trace_name}:{source_line}: invalid hex byte at offset {offset}")
                })
            })
            .collect()
    }

    fn normalized_primary(terminal: &TerminalBackend) -> NormalizedSnapshot {
        let snapshot = terminal
            .snapshot()
            .expect("in-memory terminal snapshot is valid");
        NormalizedSnapshot {
            columns: snapshot.size().columns(),
            rows: snapshot.size().rows(),
            cursor: snapshot.cursor(),
            modes: NormalizedModes {
                alternate_screen: snapshot.modes().alternate_screen,
                application_cursor: snapshot.modes().application_cursor,
                application_keypad: snapshot.modes().application_keypad,
                bracketed_paste: snapshot.modes().bracketed_paste,
                cursor_visible: snapshot.modes().cursor_visible,
                mouse_enabled: snapshot.modes().mouse != crate::snapshot::MouseMode::Disabled,
            },
            viewport: (0..snapshot.row_count())
                .map(|row| NormalizedRow {
                    cells: (0..usize::from(snapshot.size().columns()))
                        .map(|column| {
                            let cell = snapshot
                                .cell(row, column)
                                .expect("snapshot has each visible cell");
                            NormalizedCell {
                                contents: cell.contents().to_owned(),
                                wide: cell.is_wide(),
                                wide_continuation: cell.is_wide_continuation(),
                                attributes: cell.attributes().clone(),
                            }
                        })
                        .collect(),
                    wrapped: snapshot
                        .row_wrapped(row)
                        .expect("snapshot has each visible row"),
                })
                .collect(),
        }
    }

    fn normalized_alacritty(
        terminal: &AlacrittyTerm<VoidListener>,
        size: Size,
    ) -> NormalizedSnapshot {
        let grid = terminal.grid();
        NormalizedSnapshot {
            columns: size.columns(),
            rows: size.rows(),
            cursor: crate::snapshot::Cursor {
                row: u16::try_from(grid.cursor.point.line.0)
                    .expect("Alacritty cursor remains visible"),
                column: u16::try_from(grid.cursor.point.column.0)
                    .expect("Alacritty cursor fits terminal width"),
                visible: terminal.mode().contains(TermMode::SHOW_CURSOR),
            },
            modes: NormalizedModes {
                alternate_screen: terminal.mode().contains(TermMode::ALT_SCREEN),
                application_cursor: terminal.mode().contains(TermMode::APP_CURSOR),
                application_keypad: terminal.mode().contains(TermMode::APP_KEYPAD),
                bracketed_paste: terminal.mode().contains(TermMode::BRACKETED_PASTE),
                cursor_visible: terminal.mode().contains(TermMode::SHOW_CURSOR),
                mouse_enabled: terminal.mode().intersects(TermMode::MOUSE_MODE),
            },
            viewport: (0..usize::from(size.rows()))
                .map(|row| {
                    let alacritty_row = AlacrittyLine(row as i32);
                    let cells: Vec<_> = (0..usize::from(size.columns()))
                        .map(|column| {
                            normalize_alacritty_cell(&grid[alacritty_row][AlacrittyColumn(column)])
                        })
                        .collect();
                    let wrapped = grid[alacritty_row]
                        [AlacrittyColumn(usize::from(size.columns()) - 1)]
                    .flags
                    .contains(Flags::WRAPLINE);
                    NormalizedRow { cells, wrapped }
                })
                .collect(),
        }
    }

    fn normalize_alacritty_cell(cell: &AlacrittyCell) -> NormalizedCell {
        let wide_continuation = cell.flags.contains(Flags::WIDE_CHAR_SPACER);
        // `vt100` represents a default blank as an empty string while
        // Alacritty stores a space. They are the same visible cell.
        let mut contents = if wide_continuation || cell.c == ' ' {
            String::new()
        } else {
            cell.c.to_string()
        };
        if !wide_continuation && let Some(zerowidth) = cell.zerowidth() {
            contents.extend(zerowidth);
        }
        NormalizedCell {
            contents,
            wide: cell.flags.contains(Flags::WIDE_CHAR),
            wide_continuation,
            attributes: crate::snapshot::CellAttributes {
                foreground: normalize_alacritty_color(cell.fg),
                background: normalize_alacritty_color(cell.bg),
                bold: cell.flags.contains(Flags::BOLD),
                dim: cell.flags.contains(Flags::DIM),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                inverse: cell.flags.contains(Flags::INVERSE),
            },
        }
    }

    fn normalize_alacritty_color(color: AlacrittyColor) -> crate::snapshot::Color {
        match color {
            AlacrittyColor::Indexed(index) => crate::snapshot::Color::Indexed(index),
            AlacrittyColor::Spec(rgb) => crate::snapshot::Color::Rgb(rgb.r, rgb.g, rgb.b),
            AlacrittyColor::Named(named) => match named {
                NamedColor::Black => crate::snapshot::Color::Indexed(0),
                NamedColor::Red => crate::snapshot::Color::Indexed(1),
                NamedColor::Green => crate::snapshot::Color::Indexed(2),
                NamedColor::Yellow => crate::snapshot::Color::Indexed(3),
                NamedColor::Blue => crate::snapshot::Color::Indexed(4),
                NamedColor::Magenta => crate::snapshot::Color::Indexed(5),
                NamedColor::Cyan => crate::snapshot::Color::Indexed(6),
                NamedColor::White => crate::snapshot::Color::Indexed(7),
                NamedColor::BrightBlack => crate::snapshot::Color::Indexed(8),
                NamedColor::BrightRed => crate::snapshot::Color::Indexed(9),
                NamedColor::BrightGreen => crate::snapshot::Color::Indexed(10),
                NamedColor::BrightYellow => crate::snapshot::Color::Indexed(11),
                NamedColor::BrightBlue => crate::snapshot::Color::Indexed(12),
                NamedColor::BrightMagenta => crate::snapshot::Color::Indexed(13),
                NamedColor::BrightCyan => crate::snapshot::Color::Indexed(14),
                NamedColor::BrightWhite => crate::snapshot::Color::Indexed(15),
                _ => crate::snapshot::Color::Default,
            },
        }
    }

    fn preserve_differential_failure(
        trace: &CanonicalTrace,
        operation_index: usize,
        description: &str,
        event_log: &[String],
        primary: &NormalizedSnapshot,
        reference: &NormalizedSnapshot,
    ) -> PathBuf {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/ptytest-differential")
            .join(format!("{}-{}", trace.name, std::process::id()));
        let _ = fs::create_dir_all(&directory);
        let metadata = format!(
            "trace={}\noperation={}\nevent={}\nprimary=vt100 0.16.2\nreference=alacritty_terminal 0.26.0\n\nevents:\n{}\n",
            trace.name,
            operation_index,
            description,
            event_log.join("\n"),
        );
        let _ = fs::write(directory.join("metadata.txt"), metadata);
        let _ = fs::write(
            directory.join("vt100-normalized.txt"),
            format!("{primary:#?}\n"),
        );
        let _ = fs::write(
            directory.join("alacritty-normalized.txt"),
            format!("{reference:#?}\n"),
        );
        directory
    }

    fn hex(bytes: &[u8]) -> String {
        let mut encoded = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            encoded.push_str(&format!("{byte:02x}"));
        }
        encoded
    }

    #[test]
    fn canonical_primary_snapshots_are_platform_stable() {
        assert_eq!(
            canonical_primary_text(),
            include_str!("../tests/fixtures/differential/canonical-vt100.ptytest"),
        );
    }

    // Every recorded output chunk is independently split at every byte
    // boundary. This is separate from input-fragmentation testing: it proves
    // the terminal model and the audited output profile both retain their
    // contract when kernel reads divide a real consumer fragment differently.
    #[test]
    fn canonical_corpus_is_chunk_boundary_independent_and_profile_valid() {
        for trace in CANONICAL_TRACES {
            let expected = replay_primary_with_profile(trace, None);
            for (operation_index, operation) in parse_trace(trace).iter().enumerate() {
                let DifferentialOperation::Bytes { bytes, .. } = operation else {
                    continue;
                };
                for split in 0..=bytes.len() {
                    assert_eq!(
                        replay_primary_with_profile(trace, Some((operation_index, split))),
                        expected,
                        "{} operation {operation_index} split {split}",
                        trace.name,
                    );
                }
            }
        }
    }

    fn canonical_primary_text() -> String {
        let mut output = String::new();
        for trace in CANONICAL_TRACES {
            let mut terminal = TerminalBackend::new(
                Size::new(trace.columns, trace.rows).expect("canonical trace has a valid size"),
            );
            for operation in parse_trace(trace) {
                match operation {
                    DifferentialOperation::Bytes { bytes, .. } => terminal.process(&bytes),
                    DifferentialOperation::Resize { columns, rows, .. } => {
                        terminal.resize(
                            Size::new(columns, rows).expect("canonical trace has a valid resize"),
                        );
                    }
                }
            }
            output.push_str(&format!(
                "== {} ==\n{}",
                trace.name,
                terminal.snapshot().expect("terminal snapshot is valid")
            ));
        }
        output
    }

    fn replay_primary_with_profile(
        trace: &CanonicalTrace,
        split: Option<(usize, usize)>,
    ) -> ScreenSnapshot {
        let mut terminal = TerminalBackend::new(
            Size::new(trace.columns, trace.rows).expect("canonical trace has a valid size"),
        );
        let mut peer = TerminalPeer::new(ProtocolProfile::xterm_minimal_v1());
        for (operation_index, operation) in parse_trace(trace).into_iter().enumerate() {
            match operation {
                DifferentialOperation::Bytes { bytes, .. } => {
                    let split_at = split
                        .filter(|(index, _)| *index == operation_index)
                        .map(|(_, split_at)| split_at)
                        .unwrap_or(bytes.len());
                    for chunk in [&bytes[..split_at], &bytes[split_at..]] {
                        for byte in chunk {
                            let byte = [*byte];
                            terminal.process(&byte);
                            let cursor = terminal.state().expect("terminal state is valid").cursor;
                            assert!(
                                peer.observe(&byte, (cursor.row, cursor.column))
                                    .expect("canonical profile output is valid")
                                    .is_empty(),
                                "canonical traces do not contain terminal queries",
                            );
                        }
                    }
                }
                DifferentialOperation::Resize { columns, rows, .. } => {
                    terminal.resize(
                        Size::new(columns, rows).expect("canonical trace has a valid resize"),
                    );
                }
            }
        }
        peer.finish()
            .expect("canonical profile control is complete");
        terminal.snapshot().expect("terminal snapshot is valid")
    }
}
