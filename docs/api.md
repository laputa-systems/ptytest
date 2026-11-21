# Public API

The Cargo package is `laputa-ptytest`; Rust callers import the library as `ptytest`. Public item documentation in `src/lib.rs` and the exported types is the complete signature reference.

## Scenarios and child environment

`Scenario::new` requires a stable, valid label, defaults to an 80 by 24 terminal, and uses a hermetic environment when none is supplied. The label identifies failure artifacts and is sanitized for its directory name. `CommandSpec` owns the executable path, arguments, working directory, and environment edits; it uses direct execution without shell parsing or implicit `PATH` lookup.

`Size::new(columns, rows)` rejects zero dimensions. `TestEnv::hermetic()` clears inherited application variables and supplies isolated home, XDG, temporary, and fixture paths with `TERM=xterm-256color`, `PATH=/usr/bin:/bin`, `LANG=C`, `LC_ALL=C`, and `TZ=UTC`. `hermetic_utf8(locale)` sets both locale variables and errors if the host does not report that locale; use `C.UTF-8` where available for portable Unicode tests. `secret_env` redacts text diagnostics; raw input and output remain unredacted.

## Driving a terminal

`PtyTest` owns one master descriptor, child process identity, terminal model and peer, trace, and scenario. It is neither `Clone` nor `Sync`.

- `send_bytes`, `send_text`, and `send_key` write to the PTY. `send_key` uses the observed terminal mode for supported mode-sensitive encodings; drain pending output and wait for the relevant mode before encoding a key. Use bytes when the exact wire representation is required.
- `send_eof` writes the configured terminal EOF character and requires canonical input mode. `Key::Ctrl('d')` sends a byte instead. `hangup` closes the master in both directions.
- `resize` changes the kernel window size with `TIOCSWINSZ`. `signal` targets the child process group.
- Screen and exit waits, `drain`, and output-quiescence waits take a monotonic caller-supplied `Deadline`. `wait_for_quiescence` returns `true` only after the full quiet interval passes without output; it returns `false` if the overall deadline expires first. Reads and writes check the deadline while making progress, and interrupted polls recalculate the remaining time.

### Synchronized output

`screen`, `terminal_state`, `wait_for_screen`, `wait_for_screen_change`, and `assert_snapshot` observe the screen as a terminal presents it. When the child sets DEC private mode 2026 (`CSI ? 2026 h`), they keep showing the screen, cursor, and modes from before the update until the child resets the mode (`CSI ? 2026 l`); the whole update then appears at once. This removes torn observations of a frame the child writes in several PTY reads. `wait_for_quiescence` and `raw_output` are byte-level and unchanged, so quiescence can return while an update is still held; the screen it then shows is the held one.

- Recognition follows the VT parser, so a `h` or `l` sequence split across reads works, and a combined list such as `CSI ? 25 ; 2026 h` begins the update. Parameters apply in order: a mode listed before 2026 is part of the held frame, a mode listed after it is not.
- Updates do not nest. A repeated `h` keeps the first held frame, and one `l` ends the update; an `l` with no update pending is ignored.
- A resize during an update resizes the held screen as well.
- `send_key` and terminal-query replies use the child's latest modes and cursor, not the held ones, as a real terminal would. `assert_terminal_restored` compares the latest modes.
- If no `l` ever arrives, the update is abandoned and the screen shows the latest state when the PTY reaches end of file or hangs up, when `hangup` is called, and when `finish` completes. `wait_for_exit` reads once more after seeing the exit so a child that exits mid-update is released deterministically. A child that stays alive and never ends the update leaves the screen held: there is no timeout. Real terminals release after roughly 150 ms to 1 s, but a wall-clock release would expose a torn frame whenever the writer or the test is descheduled for that long, which is the failure this feature removes. Every wait already has a caller deadline, and a timeout bundle then includes `live-screen.ptytest` next to the held `screen.ptytest`.
- `synchronized_update_pending` reports whether an update is open and `live_screen` returns the latest state, for diagnosing a child that never ends one.

`ScreenSnapshot` is an owned semantic view, not a `vt100` type. Its text and row helpers ignore diagnostic whitespace markers. `TerminalState` reports only supported model state. `terminal_baseline` and `assert_terminal_restored` compare applicable application-owned lifecycle state, not screen history or contents. `wait_for_exit` observes without reaping; `finish` reaps and returns an `ExitStatus` that distinguishes an exit code, a signal, a running process, or an unreaped process. Callers must check the returned status.

Keep application-specific prompt, command, and provider helpers in the consumer's tests; they compose from this general PTY API.

See [failure artifacts](failure-artifacts.md) for snapshot updating and failure output, and [backend capabilities](backend-capabilities.md) for semantic-model limits.
