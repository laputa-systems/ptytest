Prefix: PTY
Next: 3

### PTY-2 · P3 · Differential oracle collapses mouse mode to a boolean

- **Area:** Tests
- **Where:** `src/terminal.rs:504`, `src/terminal.rs:60`, `tests/fixtures/differential/mode-lifecycle.trace:2`
- **Problem:** `TerminalModes.mouse` is a five-value enum mapped from `vt100::MouseProtocolMode`, but `normalized_primary` records only `mouse_enabled`. The mode-lifecycle trace sends `CSI ?1000h` and `CSI ?1006h`, and no test asserts `MouseMode::Press`, `PressRelease`, `ButtonMotion`, or `AnyMotion`. A swapped mapping still passes the corpus comparison whenever both backends leave mouse tracking on.
- **Recommendation:** Compare the mouse-mode enum in the differential snapshot, and assert the mode `CSI ?1000h` produces. Keep SGR 1006 as an encoding flag only if the backend does not treat it as a `MouseMode`.
- **Fix with:** test-audit
- **Effort:** S
