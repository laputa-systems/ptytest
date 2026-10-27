Prefix: PTY
Next: 3

### PTY-1 · P2 · Spawned programs inherit the parent's descriptors and ignored SIGPIPE

- **Area:** Concurrency
- **Where:** `src/unix.rs:519` (`child_exec`), `src/unix.rs:38` (`spawn`)
- **Problem:** `child_exec` dupes the slave onto standard descriptors and calls `execve`, and it does not close or mark close-on-exec any other descriptor and does not restore `SIGPIPE`. A probe on macOS spawned this crate's `PtyTest` with a non-cloexec pipe open in the parent. The child printed `FD_FLAGS=0` for that descriptor (`F_GETFD` succeeded) and `SIGPIPE_PREV=1`, which is `SIG_IGN` (`SIG_DFL` is 0). Rust installs `SIG_IGN` for `SIGPIPE`, and an ignored signal survives `execve`. The program under test can read and write the test process's non-cloexec descriptors, hold a pipe open so the parent never sees EOF, and keep running after a broken pipe instead of dying with signal 13. `std::process::Command` closes extra descriptors and restores `SIGPIPE`. This path does not.
- **Recommendation:** In the child, after the standard descriptors are duped, set close-on-exec or close every descriptor above stderr, and set `SIGPIPE` to `SIG_DFL` before `execve`. Keep the spawn error pipe usable until `execve`. Add a regression that inherits one non-cloexec pipe and asserts the child cannot `fcntl` it, and one that asserts `SIGPIPE` is `SIG_DFL`.
- **Fix with:** lifecycle-concurrency
- **Effort:** S

### PTY-2 · P3 · Differential oracle collapses mouse mode to a boolean

- **Area:** Tests
- **Where:** `src/terminal.rs:504`, `src/terminal.rs:60`, `tests/fixtures/differential/mode-lifecycle.trace:2`
- **Problem:** `TerminalModes.mouse` is a five-value enum mapped from `vt100::MouseProtocolMode`, but `normalized_primary` records only `mouse_enabled`. The mode-lifecycle trace sends `CSI ?1000h` and `CSI ?1006h`, and no test asserts `MouseMode::Press`, `PressRelease`, `ButtonMotion`, or `AnyMotion`. A swapped mapping still passes the corpus comparison whenever both backends leave mouse tracking on.
- **Recommendation:** Compare the mouse-mode enum in the differential snapshot, and assert the mode `CSI ?1000h` produces. Keep SGR 1006 as an encoding flag only if the backend does not treat it as a `MouseMode`.
- **Fix with:** test-audit
- **Effort:** S
