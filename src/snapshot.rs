use crate::Size;
use std::fmt;

/// An immutable semantic view of one visible terminal cell.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellSnapshot {
    contents: String,
    wide: bool,
    wide_continuation: bool,
    attributes: CellAttributes,
}

impl CellSnapshot {
    pub fn contents(&self) -> &str {
        &self.contents
    }
    pub fn is_wide(&self) -> bool {
        self.wide
    }
    pub fn is_wide_continuation(&self) -> bool {
        self.wide_continuation
    }
    pub fn attributes(&self) -> &CellAttributes {
        &self.attributes
    }
}

/// Visible cell styling, independent of the terminal-model implementation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CellAttributes {
    pub foreground: Color,
    pub background: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl CellAttributes {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// A terminal colour represented without exposing `vt100` types.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The visible screen and the supported lifecycle modes at one instant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenSnapshot {
    size: Size,
    cursor: Cursor,
    modes: TerminalModes,
    rows: Vec<Vec<CellSnapshot>>,
    row_wrapped: Vec<bool>,
}

impl ScreenSnapshot {
    pub(crate) fn new(
        size: Size,
        cursor: Cursor,
        modes: TerminalModes,
        rows: Vec<Vec<CellSnapshot>>,
        row_wrapped: Vec<bool>,
    ) -> Self {
        Self {
            size,
            cursor,
            modes,
            rows,
            row_wrapped,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }
    pub fn modes(&self) -> &TerminalModes {
        &self.modes
    }
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
    pub fn row_wrapped(&self, row: usize) -> Option<bool> {
        self.row_wrapped.get(row).copied()
    }
    pub fn cell(&self, row: usize, column: usize) -> Option<&CellSnapshot> {
        self.rows.get(row).and_then(|cells| cells.get(column))
    }

    /// Returns a row as a person reads it: cells the application never wrote
    /// read as spaces, wide continuation cells are elided, and trailing
    /// unwritten cells are dropped. Applications that repaint only changed
    /// cells skip over blanks with cursor movement, so those gaps must not
    /// collapse words together. This is the convenient assertion view, not the
    /// golden format.
    pub fn row(&self, row: usize) -> Option<String> {
        self.rows.get(row).map(|cells| {
            let written = cells
                .iter()
                .rposition(|cell| !cell.contents.is_empty() || cell.wide_continuation)
                .map_or(0, |last| last + 1);
            cells[..written]
                .iter()
                .filter(|cell| !cell.wide_continuation)
                .map(|cell| if cell.contents.is_empty() { " " } else { cell.contents.as_str() })
                .collect()
        })
    }

    /// Searches visible text without making callers account for diagnostic
    /// whitespace markers.
    pub fn contains(&self, needle: &str) -> bool {
        self.rows.iter().enumerate().any(|(row, _)| {
            self.row(row)
                .is_some_and(|contents| contents.contains(needle))
        })
    }

    pub fn to_text(&self, options: SnapshotOptions) -> String {
        let mut output = format!(
            "size: columns={} rows={}\ncursor: row={} column={} visible={}\nmodes: alternate-screen={} application-cursor={} application-keypad={} bracketed-paste={}\n",
            self.size.columns(),
            self.size.rows(),
            self.cursor.row,
            self.cursor.column,
            self.cursor.visible,
            self.modes.alternate_screen,
            self.modes.application_cursor,
            self.modes.application_keypad,
            self.modes.bracketed_paste,
        );
        for (index, cells) in self.rows.iter().enumerate() {
            output.push_str(&format!("{index:02}│"));
            for cell in cells {
                if cell.wide_continuation {
                    output.push('›');
                } else if cell.contents == " " || cell.contents.is_empty() {
                    output.push('·');
                } else {
                    output.push_str(&cell.contents);
                }
            }
            output.push('\n');
        }
        if options.attributes {
            for (row, cells) in self.rows.iter().enumerate() {
                let mut start = None;
                let mut attributes = None;
                for (column, cell) in cells
                    .iter()
                    .enumerate()
                    .chain(std::iter::once((cells.len(), &default_cell())))
                {
                    let current = (!cell.attributes.is_default()).then_some(&cell.attributes);
                    if current != attributes.as_ref() {
                        if let (Some(start), Some(previous)) = (start.take(), attributes.take()) {
                            output.push_str(&format!(
                                "attr: row={row} columns={start}..{column} {}\n",
                                attributes_text(&previous)
                            ));
                        }
                        start = current.map(|_| column);
                        attributes = current.cloned();
                    }
                }
            }
        }
        output
    }
}

fn default_cell() -> CellSnapshot {
    CellSnapshot {
        contents: String::new(),
        wide: false,
        wide_continuation: false,
        attributes: CellAttributes::default(),
    }
}

fn attributes_text(attributes: &CellAttributes) -> String {
    format!(
        "fg={:?} bg={:?} bold={} dim={} italic={} underline={} inverse={}",
        attributes.foreground,
        attributes.background,
        attributes.bold,
        attributes.dim,
        attributes.italic,
        attributes.underline,
        attributes.inverse
    )
}

impl fmt::Display for ScreenSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_text(SnapshotOptions::default()))
    }
}

/// Options controlling the stable textual snapshot representation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SnapshotOptions {
    attributes: bool,
}

impl SnapshotOptions {
    pub fn with_attributes(mut self) -> Self {
        self.attributes = true;
        self
    }
}

/// A cursor in zero-based screen coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub row: u16,
    pub column: u16,
    pub visible: bool,
}

/// Input and lifecycle modes faithfully represented by the selected backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalModes {
    pub alternate_screen: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub bracketed_paste: bool,
    pub cursor_visible: bool,
    pub mouse: MouseMode,
}

/// The represented xterm mouse-reporting mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MouseMode {
    #[default]
    Disabled,
    Press,
    PressRelease,
    ButtonMotion,
    AnyMotion,
}

/// Focused model state. Fields absent here are intentionally not fabricated
/// when the selected backend cannot report them faithfully.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalState {
    pub size: Size,
    pub cursor: Cursor,
    pub modes: TerminalModes,
    pub row_wrapped: Vec<bool>,
    pub scrollback_offset: usize,
}

impl TerminalState {
    pub(crate) fn new(
        size: Size,
        cursor: Cursor,
        modes: TerminalModes,
        row_wrapped: Vec<bool>,
        scrollback_offset: usize,
    ) -> Self {
        Self {
            size,
            cursor,
            modes,
            row_wrapped,
            scrollback_offset,
        }
    }
}

/// Applicable lifecycle state captured from the initial terminal model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalBaseline(pub(crate) TerminalState);

impl TerminalBaseline {
    pub fn state(&self) -> &TerminalState {
        &self.0
    }
}

pub(crate) fn cell_snapshot(
    contents: String,
    wide: bool,
    wide_continuation: bool,
    attributes: CellAttributes,
) -> CellSnapshot {
    CellSnapshot {
        contents,
        wide,
        wide_continuation,
        attributes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_read_unwritten_gaps_as_spaces_and_drop_trailing_blanks() {
        let mut terminal = crate::terminal::TerminalBackend::new(Size::new(20, 2).unwrap());
        // A repaint that skips blanks with cursor movement leaves cells that
        // were never written between the words.
        terminal.process(b"ab\x1b[3Ccd\x1b[2;1H\xe7\x95\x8c!");
        let snapshot = terminal.snapshot().unwrap();
        assert_eq!(snapshot.row(0).as_deref(), Some("ab   cd"));
        assert!(snapshot.contains("ab   cd"));
        assert!(!snapshot.contains("abcd"));
        assert_eq!(snapshot.row(1).as_deref(), Some("界!"));
    }

    #[test]
    fn text_snapshot_marks_blank_and_continuation_cells() {
        let size = Size::new(2, 1).unwrap();
        let snapshot = ScreenSnapshot::new(
            size,
            Cursor {
                row: 0,
                column: 0,
                visible: true,
            },
            TerminalModes {
                alternate_screen: false,
                application_cursor: false,
                application_keypad: false,
                bracketed_paste: false,
                cursor_visible: true,
                mouse: MouseMode::Disabled,
            },
            vec![vec![
                cell_snapshot("界".into(), true, false, CellAttributes::default()),
                cell_snapshot("".into(), false, true, CellAttributes::default()),
            ]],
            vec![false],
        );
        assert_eq!(snapshot.row(0).as_deref(), Some("界"));
        assert!(
            snapshot
                .to_text(SnapshotOptions::default())
                .contains("00│界›")
        );
    }
}
