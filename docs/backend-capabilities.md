# Terminal backend capabilities

`ptytest`'s v1 semantic backend is `vt100 0.18.1`, behind the private
`TerminalBackend` in `src/terminal.rs`. `ScreenSnapshot` and `TerminalState`
expose only state that this backend represents faithfully.

## Synchronized output

`vt100` ignores DEC private mode 2026, so `TerminalBackend` implements it:
it takes a copy of the screen when `CSI ? 2026 h` is reported as an unhandled
sequence and presents that copy until `CSI ? 2026 l`. The snapshot and state
accessors present the copy; the cursor used for terminal replies and the modes
used for key encoding come from the live screen. Replay uses the same backend.
See [the API notes](api.md#synchronized-output) for the abandonment rules.

## Resize and soft-wrap reflow

`vt100::Screen::set_size` preserves the existing row layout when a terminal is
resized. Mature GUI terminal emulators such as Alacritty can instead reflow
soft-wrapped logical lines, changing the post-resize cursor position and
visible rows. These are intentionally different terminal-model policies.

Consequently, v1 supports and differentially checks kernel resize and visible
size changes, including content with explicit line boundaries. It does not
claim soft-wrap reflow as a `ScreenSnapshot` or `TerminalState` contract. A
consumer whose behavior depends on reflow must assert the raw protocol/layout
contract it owns, or select a future backend with a reviewed reflow capability;
`ptytest` does not fabricate a reflow result.
