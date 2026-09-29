# AGENTS.md — ptytest

`ptytest` is a Unix-only (macOS + Linux) Rust test-support crate. It tests a
compiled interactive program at its real terminal boundary: it owns a kernel
PTY, executes the supplied binary directly (never through a shell), writes real
input bytes, resizes with `TIOCSWINSZ`, and maintains an independent `vt100`
semantic model for assertions.

Implementation and the four consumer migrations described in the original plan
are complete. This file is the maintainer guide. Detail pages live in `docs/`:

- `docs/baselines.md` — dependency authorization, pristine consumer baselines,
  platform/toolchain evidence, post-migration verification.
- `docs/protocol-profiles.md` — the `xterm-minimal-v1` rule set and its
  semantic reasons.
- `docs/protocol-inventory.md` — reviewed per-consumer trace inventory.
- `docs/backend-capabilities.md` — what the terminal backend does and does not
  claim (notably: no soft-wrap reflow).

## Boundary and guarantees

```text
                     Unix kernel PTY
                          │
      ┌───────────────────┴───────────────────┐
      │                                       │
  test harness                         application binary
      │                                       │
      │ input / replies / resize / signals    │
      ├──────────────────────────────────────> │
      │                                       │
      │ <─────────────────────────────────────┤
      │              terminal bytes            │
      │                                       │
      ├── raw event/byte trace                 │
      ├── independent VT terminal model        │
      ├── protocol-profile validator           │
      └── deterministic terminal-query responder
```

The semantic terminal model is the primary visual oracle; raw bytes are the
protocol/debugging oracle. The protocol validator is independent of the
semantic model. The responder is a deterministic terminal peer: an approved
query receives its profile-controlled reply, an unsupported query fails
explicitly instead of hanging until a generic timeout.

Enduring guarantees:

- The child starts with PTY-backed stdin/stdout/stderr, a controlling
  terminal, a fresh session/process group, foreground-process-group ownership,
  termios, and an initial kernel window size.
- Tests send bytes through the master and resize through `TIOCSWINSZ`; they
  never invoke application input or resize internals.
- Every operation that may wait takes a monotonic caller-supplied `Deadline`.
  Sleeps are never a synchronization primitive.
- Output is interpreted incrementally: arbitrary chunk boundaries, split escape
  sequences, and split UTF-8 are all valid.
- Timeouts and crate-owned assertions capture a readable screen, raw
  input/output, event history, configuration, and process state. Dropping a
  live harness during unwinding writes the same bundle before cleanup.
- Cleanup is bounded and targets the original child process group: it reaps
  the child and ordinary descendants still in that group. It does not promise
  to kill a descendant that daemonizes into another session, and it never
  demands terminal restoration after `SIGKILL`.

Out of scope: Windows/ConPTY, remote connections, shell-command parsing,
GUI/pixel output, fonts, mouse abstraction beyond raw-byte helpers,
recording/GIF output, async APIs, and differential emulator testing on every
PR. `ptytest` serves PTY integration and protocol conformance; ordinary
non-interactive CLI tests stay plain pipe/subprocess tests, and application
unit tests stay where they are.

## Package, toolchain, layout

- Package `laputa-ptytest`, library target `ptytest` (`use ptytest::...`).
  Pinned toolchain in `rust-toolchain.toml`; crate is gated `#![cfg(unix)]`
  with an intentional compile error elsewhere.
- Runtime dependencies: `libc`, `vt100 = 0.16.2` only. Test-only differential
  oracle: `alacritty_terminal`. Adding any dependency requires explicit user
  approval first — never substitute a larger PTY abstraction silently.
- Layout: `src/lib.rs` (public types/docs only), `src/config.rs`
  (`CommandSpec`, `Scenario`, `TestEnv`, `Size`, deadlines),
  `src/unix.rs` (PTY/spawn/resize/signal/reap — the only module with platform
  FFI detail), `src/io.rs` (nonblocking I/O, poll loop, event timeline),
  `src/terminal.rs` (private backend boundary), `src/snapshot.rs` (text
  format/comparison), `src/protocol.rs` (profiles, queries, responder),
  `src/artifact.rs` (failure bundles), `src/replay.rs` plus
  `src/bin/ptytest-replay.rs` (offline replay), `tests/support/` (deterministic
  fixture binary), `tests/` (acceptance/invariant tests).

## Public API

```rust
use ptytest::{CommandSpec, Key, PtyTest, Scenario, Size, TestEnv};
use std::time::Duration;

# fn example() -> Result<(), ptytest::PtyTestError> {
let scenario = Scenario::new("startup")?
    .command(CommandSpec::new("/path/to/example"))
    .size(Size::new(100, 24)?)
    .environment(TestEnv::hermetic()?);
let mut terminal = PtyTest::spawn(scenario)?;
let baseline = terminal.terminal_baseline();

terminal.wait_for_screen(terminal.deadline(Duration::from_secs(3)), "project list", |screen| {
    screen.contains("Projects")
})?;
terminal.send_key(terminal.deadline(Duration::from_secs(3)), Key::Down)?;

terminal.resize(Size::new(40, 10)?)?;
terminal.wait_for_screen(terminal.deadline(Duration::from_secs(3)), "narrow layout", |screen| {
    screen.size() == Size::new(40, 10).expect("constant size")
})?;
terminal.send_key(terminal.deadline(Duration::from_secs(3)), Key::Ctrl('c'))?;
terminal.wait_for_exit(terminal.deadline(Duration::from_secs(3)))?;
terminal.assert_terminal_restored(&baseline)?;
terminal.finish(terminal.deadline(Duration::from_secs(3)))?;
# Ok(()) }
```

Type and ownership rules:

- `Size { columns, rows }` is non-zero at construction, terminal ordering
  (columns first). Kernel rows/columns conversion lives only in the Unix
  boundary.
- `CommandSpec` owns program, args, cwd, and env additions/removals; it
  executes via `execve` with no shell and no implicit `PATH` lookup.
- `Scenario` needs a stable caller-provided label (sanitized for artifact
  paths; empty/invalid is an error). It determines artifact directories and
  snapshot-failure identity.
- `TestEnv::hermetic()` clears inherited app env and creates a unique scratch
  root (`home`, `config`, `data`, `cache`, `state`, `tmp`, `fixtures`) with
  `TERM=xterm-256color`, `PATH=/usr/bin:/bin`, `LANG=C`, `LC_ALL=C`, `TZ=UTC`.
  `hermetic_utf8(locale)` sets `LANG`/`LC_ALL` together and fails explicitly
  if the locale is unavailable; portable Unicode locale is `C.UTF-8`.
  Inheritance is opt-in; `secret_env` redacts text diagnostics only.
- `PtyTest` solely owns master, child PID/group, terminal peer/model, raw
  buffers, event trace, config, artifact policy. Neither `Clone` nor `Sync`.
- `send_bytes` / `send_text` / `send_key` / `send_fragmented` write the real
  byte stream. `send_key` derives mode-sensitive encodings from current
  terminal state (e.g. DECCKM application-cursor) and reports unsupported
  encodings instead of guessing; `Bytes` / `send_bytes` are the exact
  wire-level escape hatches. Drain-before-encode, and wait for a mode
  predicate before relying on a new mode.
- `send_eof` writes the termios EOF char (canonical discipline only);
  `Key::Ctrl('d')` stays independent. `hangup` closes the master (both
  directions). There is no generic half-close API.
- `resize` issues `TIOCSWINSZ` only — no app calls, no synthetic `SIGWINCH`.
  `signal` targets the child process group for lifecycle tests.
- `Deadline` is `Instant`-based. `wait_for_screen`,
  `wait_for_screen_change`, `drain`, `wait_for_exit` all require one.
- `ExitStatus` distinguishes `Code`, `Signal`, `Running`, `Unreaped`.
  `wait_for_exit` observes without relinquishing group identity; `finish` /
  `Drop` reap. Never assert success silently.
- `ScreenSnapshot` is the owned semantic view (cells, cursor, visible
  attributes, selected lifecycle modes), safe to retain, never a public
  `vt100` alias. `TerminalState` is the focused lifecycle/model query view and
  never invents state the backend cannot represent.
- `terminal_baseline` / `assert_terminal_restored` compare only
  applicable application-owned lifecycle state, excluding history/contents.
- `PtyTestError` is structured; every timeout/assertion reports scenario,
  operation/predicate, monotonic elapsed time, status, dimensions, semantic
  screen, recent events, protocol violation if any, and artifact path. Library
  calls return `Result`; tests decide between `?` / `expect` / assertions.
- Never add app-specific helpers (`wait_for_prompt`, `run_command`,
  fixtures for providers/models). Those compose from this API in the consumer.

## I/O, spawn, cleanup contracts

- Master writes handle partial writes, `EINTR`, `EAGAIN`/`EWOULDBLOCK` with
  `poll(POLLOUT)` until the deadline. Reads handle `POLLIN`/`POLLHUP`/
  `POLLERR`/`POLLNVAL`, `EINTR`, `EAGAIN`/`EWOULDBLOCK`, EOF, and platform
  `EIO` slave-close; HUP/ERR with readable data drains through `EAGAIN`
  first so final restoration bytes survive. Every chunk feeds raw output, the
  model, and the profile scanner unchanged.
- Spawn: `openpty` at the requested size, close-on-exec on harness fds,
  pre-`fork` exec preparation, then a reviewed post-`fork` sequence only
  (close master, `setsid`, controlling terminal, `tcsetpgrp` + `tcgetpgrp`
  verify, dup slave to 0/1/2, `chdir`, `execve`; failures write a fixed error
  record and `_exit` with no allocation/locking/formatting). The parent reads
  the error pipe to separate spawn failure from app exit and retains the
  session-leading PID as group identity.
- `Drop` is a safety net: records artifacts first, observes (non-consuming)
  status where supported, signals the verified original group `SIGTERM`,
  polls, escalates once to `SIGKILL`, then reaps. After reaping it never
  signals the numeric group ID again (PID reuse). Linux `openpty` linking and
  Darwin `libproc` group fallback stay inside `src/unix.rs`.
- Test-author rule: synchronize with screen/predicate barriers or fixture
  protocol, never delays. Genuinely elapsed-time behavior gets a controllable
  app clock or a domain-event assertion, not a generic harness wait.

## Snapshots, terminal peer, protocol profiles

- Backend is `vt100` behind a private capability boundary with an explicit
  capability record; gaps become raw-protocol assertions, model-layer
  augmentations, or a reason to pick a stronger backend — never fabricated
  state. `SnapshotOptions::with_attributes()` adds sorted coalesced
  non-default attribute ranges; the default golden stays visible semantics
  plus selected lifecycle modes. `contains`/row helpers ignore diagnostic
  whitespace markers.
- `assert_snapshot(path)` compares the owned text form; normal runs never
  write. Only `PTYTEST_UPDATE_SNAPSHOTS=1 cargo test ...` updates (atomically),
  with an inline diff plus expected/actual/diff artifacts on mismatch. Keep
  goldens beside their owning scenario; prefer focused cell/style assertions
  where they say more than a whole screen.
- The peer recognizes admitted queries (DA, DSR, CPR) incrementally and
  replies deterministically through the master; profiles deny replies by
  default. Detection, reply generation, state mutation, and validation stay
  separate responsibilities. Absent-from-profile queries fail immediately as
  `UnsupportedTerminalQuery` with byte offset and control details.
- Profiles are normative compatibility contracts, never inferred from `TERM`
  or auto-expanded from traces. Default profile validates nothing and replies
  to nothing; `ProtocolProfile::xterm_minimal_v1()` admits only the reviewed
  families in `docs/protocol-profiles.md`. New output controls fail at their
  exact offset. Enabling a profile for a consumer follows its own trace
  inventory review (`docs/protocol-inventory.md`). Input fragmentation
  (app-decoder invariant) and output fragmentation (model/profile invariant)
  are tested separately.

## Failures, replay, process handling

Failure bundles (default-on, only written on failure) land in
`target/ptytest-failures/<sanitized-scenario>-<pid>-<counter>/`:
`command.txt` (redacted), `configuration.txt`, `events.jsonl` (canonical
versioned ordered trace with byte ranges), `events.log`, exact
`input.bin`/`output.bin`, `screen.ptytest`, `terminal-state.txt`,
`exit-status.txt`, `expected.ptytest` / `actual.ptytest` / `diff.txt` on
snapshot mismatch, `failure.txt`.

- Raw buffers have a configurable cap; overflow is a visible
  `TraceLimitExceeded`, never silent truncation. Text diagnostics redact
  `secret_env` values and secret-looking keys; raw `input.bin`/`output.bin`
  stay exact (use fake credentials in fixtures).
- `PtyTest::finish` escalates `SIGTERM` → `SIGKILL` within the deadline, keeps
  an observed zombie leader until group cleanup completes, then reaps.
- Offline replay (never re-executes the app): `cargo run --bin ptytest-replay
  -- target/ptytest-failures/<bundle>` with `--dump-events`, `--step`,
  `--event N`, `--byte O`. It validates contiguous byte ranges and
  query/reply traffic.

## Working rules

- Confirm macOS and Linux separately; Linux success is not macOS evidence.
  Pinned toolchain + CI runs unit and PTY acceptance on both, including the
  UTF-8 locale check, under normal and `-- --test-threads=1` scheduling.
- No `thread::sleep` for synchronization; seeded schedule/replay tests persist
  seeds and operation traces and promote real defects to named regressions.
- Do not run formatters, linters, pre-commit hooks, or push as part of this
  work; the user handles those independently.
- Keep changes coherent with few dependencies, narrow private modules, and
  precise types that make invalid states hard to express. Contract changes
  (API, types, schemas, permissions, state transitions, invariants,
  transaction boundaries, dependencies) must be reflected in code, tests, and
  docs together.
