# ptytest: executable design and migration plan

## Status and decision gate

This document is the implementation contract for a new Rust crate named
`ptytest`. It replaces the four private real-PTY test harnesses examined below.
It is intentionally Unix-only: macOS and Linux are supported; Windows, a
terminal-emulator GUI, browser automation, `script(1)`, Expect, tmux, and shell
wrappers are out of scope.

No crate code or dependency change is authorized by this plan alone. Before
Step 1 changes `Cargo.toml`, show the user the exact dependency diff and get
approval, as required by the repository working contract. The recommended MVP
dependencies are `libc` (the narrow macOS/Linux PTY, process, and polling
boundary) and `vt100 = 0.16.2` (the already-used independent terminal model).
Do not add an async runtime, snapshot framework, terminal emulator, or general
test framework.

The final product is a reusable *test-support crate*, not a production terminal
library and not a replacement for application unit tests.

## Objective

For an actual compiled interactive binary, prove its behavior across this
boundary:

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
          │
          ├── raw event/byte trace
          ├── independent VT terminal model
          ├── protocol-profile validator
          └── deterministic terminal-query responder
```

The semantic terminal model is the primary visual oracle. Raw bytes are the
protocol/debugging oracle. The protocol validator remains independent of the
semantic model. The responder makes the harness a deterministic terminal peer:
an approved query receives its profile-controlled reply; an unsupported query
fails explicitly rather than waiting for a generic test timeout.

The finished crate must make these guarantees:

- The spawned child initially has PTY-backed stdin, stdout, and stderr; a
  controlling terminal; a fresh session/process group; foreground-process-group
  ownership; termios; and an initial kernel window size.
- Tests send actual bytes through the master and resize through
  `TIOCSWINSZ`; they never invoke application input or resize internals.
- Every operation that may wait uses a monotonic, caller-supplied deadline.
  Sleeps are not a synchronization primitive in `ptytest`.
- Output is interpreted incrementally, so arbitrary output chunk boundaries,
  split escape sequences, and split UTF-8 are valid.
- Timeouts and crate-owned assertions retain a readable screen, raw input and
  output, event history, configuration, and process state. Dropping a live
  harness during an unwinding panic writes the same bundle before cleanup.
- Cleanup is bounded and targets the original child process group. It reaps the
  child and ordinary descendants that remain in that group; it does not claim
  to kill a deliberately daemonized descendant that escapes to another session.

## Evidence from the existing harnesses

| Repository | Current boundary and strengths | Gaps that `ptytest` must remove |
| --- | --- | --- |
| `pi-agent-core-rs/crates/pi-agent-tui` | `tests/pty_streaming.rs` uses `portable-pty`, a reader thread, `vt100::Parser`, semantic marker waits, and a controlled OpenRouter streaming fixture. | One private test-only wrapper; no resize/lifecycle coverage or artifacts; fixed three-second limits; `wait_for_exit` polls with a 10 ms sleep; dev dependencies duplicate the future crate. |
| `e` | `tests/e2e/harness.rs` has the broadest API: real PTY setup, clean environment, nonblocking write, reader thread, screen/cursor/style assertions, broad editor coverage, and bounded group cleanup. | Handwritten Unix FFI/layout/linking, application-specific startup/quiescence heuristics, panic-only API, lossy recording, and `vt100 0.15`. |
| `ish` | `tests/pty.rs` has deterministic HOME/fixtures, nonblocking poll-based I/O, partial-write handling, incremental `vt100` state, prompt barriers, traces, resize, and bounded escalation. | A 3,500-line private harness with binary discovery and `$ ` semantics, many fixed sleeps, a partial ANSI stripper, direct post-fork Rust setup, and panic-only errors. |
| `tm2` | `src/pty.rs` explicitly models sessions/process groups; `tests/attached_client_pty.rs` uses a deterministic fixture, `poll`, raw byte capture, a screen barrier, resize propagation, and private socket cleanup. | Production PTY code is not a test API; the outer test harness is another `openpty` implementation with one scenario, no reusable diagnostics/snapshots, and a 100 ms polling sleep while detaching. |

All four already use a real PTY and an independent `vt100` terminal model. The
common missing layer is therefore lifecycle, I/O/deadline handling, semantic
snapshots, diagnostics, and a stable public test API—not a new application
renderer or a fake terminal.

### Compatibility correction

Do not set `TERM=vt100` or call the supported output surface “VT100” by
default. The current harnesses use `xterm-256color` (`pi-agent`, `e`, and
`ish`) or `screen-256color` (`tm2`), and current applications use extensions
such as DEC private cursor/alternate-screen modes, bracketed paste, and SGR
color. `ptytest` will start with `TERM=xterm-256color` in its hermetic
environment and will expose a named protocol profile such as
`xterm-minimal-v1`. Representative traces are discovery evidence, not the
specification: each observed family is reviewed for a semantic reason before a
versioned profile rule admits it. A profile must describe the real contract; it
must not be inferred from `TERM` or automatically expanded to fit a trace.

## Scope and non-goals

In scope:

- Direct execution of a supplied binary under a macOS/Linux PTY.
- Input bytes, mode-aware keyboard convenience encodings, deterministic
  terminal replies, resize, explicit signals/hangup, exit status, terminal
  state, snapshots, protocol validation, and artifacts.
- Hermetic environment and scratch-directory support suitable for parallel
  Cargo tests.
- A small first-party fixture binary that proves the kernel-facing behavior on
  both platforms.

Out of scope for v1:

- Windows/ConPTY, remote terminal connections, shell-command parsing, GUI or
  pixel screenshots, terminal font behavior, mouse abstraction beyond raw-byte
  helpers, recording/GIF generation, a generic async API, and differential
  terminal-emulator testing on every PR.
- Replacing the applications' pure unit/property tests, changing their UI
  behavior, or porting production `tm2::pty` code into the crate.

### Test layers

```text
Layer A: pure unit/property tests       model, layout, parser, state transitions
Layer B: ordinary subprocess tests      non-interactive CLI behavior without a tty
Layer C: PTY integration                real binary + real tty + semantic state
Layer D: protocol conformance           the same PTY trace + audited profile
Layer E: differential/nightly hardening canonical trace + second terminal model
```

`ptytest` serves Layers C-E. Keep tests at the lowest layer that proves their
contract; ordinary command-output CLI tests continue to use pipes/subprocesses.

## Stable public contract

The public API is deliberately small, concrete, and synchronous. Exact Rust
spelling can change while implementing, but the listed responsibilities and
ownership boundaries are stable.

```rust
let mut terminal = PtyTest::spawn(
    Scenario::new("startup")
        .command(CommandSpec::new(env!("CARGO_BIN_EXE_example")))
        .size(Size::new(100, 24)?)
        .environment(TestEnv::hermetic()),
)?;
let baseline = terminal.terminal_baseline();

let deadline = terminal.deadline(Duration::from_secs(3));
terminal.wait_for_screen(deadline, "startup screen", |screen| {
    screen.contains("Projects")
})?;
terminal.assert_snapshot("tests/snapshots/startup.ptytest")?;

terminal.send_key(Key::Down)?;
terminal.wait_for_screen_change(terminal.deadline(Duration::from_secs(3)))?;

terminal.resize(Size::new(40, 10)?)?;
terminal.wait_for_screen(terminal.deadline(Duration::from_secs(3)), "narrow layout", |screen| {
    screen.size() == Size::new(40, 10).expect("constant size")
})?;

terminal.send_key(Key::Ctrl('c'))?;
terminal.wait_for_exit(terminal.deadline(Duration::from_secs(3)))?;
terminal.assert_terminal_restored(&baseline)?;
```

### Types and invariants

- `Size { columns, rows }` is non-zero at construction. It uses terminal
  ordering—columns first—and converts to the kernel's rows/columns only in the
  Unix boundary. If zero-sized kernel windows prove useful for robustness,
  expose them only through a narrowly named low-level stress configuration, not
  the normal TUI `Size` API.
- `CommandSpec` owns an executable path, OS-string arguments, current
  directory, and explicit environment additions/removals. It executes that
  path with `execve` (no shell and no implicit `PATH` lookup); it never accepts
  a shell command line. Owning it makes failure artifacts reproducible without
  relying on `std::process::Command` internals.
- `Scenario` has a caller-provided, stable label. It determines the artifact
  directory and makes snapshot failures identifiable. Labels are sanitized for
  paths; an invalid/empty label is an error, not a fallback name.
- `TestEnv::hermetic_ascii()` clears inherited application environment and
  creates a unique scratch root containing `home`, `config`, `data`, `cache`,
  `state`, and `tmp`. It sets the XDG/HOME/TMPDIR paths,
  `TERM=xterm-256color`, `PATH=/usr/bin:/bin`, `LANG=C`, `LC_ALL=C`, and
  `TZ=UTC`. `hermetic()` is this default. `hermetic_utf8()` uses a
  macOS/Linux-CI-validated UTF-8 locale and sets `LANG`/`LC_ALL` together; it
  fails setup explicitly if that configured locale is unavailable. Callers may
  explicitly override or add variables; inherited environment is opt-in. The
  scratch root and a fixture subdirectory are exposed for test setup.
- `PtyTest` exclusively owns the master, child PID/process group, incremental
  terminal peer/model, raw input/output buffers, ordered event trace,
  configuration, and artifact policy. It is neither `Clone` nor `Sync`.
- `send_bytes`, `send_text`, `send_key`, and `send_fragmented` write the real
  byte stream. `send_key` derives mode-sensitive encodings from the current
  terminal state when the selected backend represents the mode faithfully:
  `Key::Up`, for example, uses normal or DECCKM/application-cursor bytes as
  appropriate. It first drains immediately available output, and tests wait for
  a mode predicate before relying on a newly emitted mode. It reports an
  unsupported-mode encoding rather than silently choosing one. Keypad support
  follows the same rule when added. An explicit encoding override and
  `send_bytes` remain exact wire-level escape hatches.
- `send_eof` writes the configured terminal EOF character only where line
  discipline makes that meaningful; it is not a raw-mode stdin half-close.
  `Key::Ctrl('d')` remains independently available. `hangup` closes the PTY
  master and therefore affects both directions and normal tty hangup behavior.
  There is no generic `close_input` API: tests name Ctrl-D, terminal EOF, or
  terminal disappearance according to the behavior they require.
- `resize` changes the master with `TIOCSWINSZ`; it does not call application
  code or separately synthesize `SIGWINCH`. The controlling terminal's normal
  foreground-process-group delivery is the behavior under test. `signal`
  explicitly targets the child process group only for lifecycle tests.
- `Deadline` is built from `Instant`, not wall-clock time. `wait_for_screen`,
  `wait_for_screen_change`, `drain`, and `wait_for_exit` all require one.
  There is no unbounded `read`, `wait`, or `sleep` API.
- `ExitStatus` distinguishes a normal code, signal termination, and an
  unreaped/running child. `wait_for_exit` observes and records the real status
  without relinquishing group identity needed by `finish`/`Drop`; those paths
  reap it. It never silently asserts success; tests assert their expected
  status.
- `ScreenSnapshot` is a project-owned, user-visible semantic view: screen
  cells, cursor, important visible attributes, and selected lifecycle modes.
  It is safe to retain after the next read and is not a public alias for
  `vt100` types. `TerminalState` is a separate model/query view for focused
  lifecycle assertions (for example active screen, faithfully represented
  input modes, wrapping, scroll region, and cursor state). It does not invent
  state the backend cannot represent, and it does not make every internal mode
  part of a human golden.
- `TerminalPeer` is a narrow internal service that observes complete terminal
  controls, feeds the independent model and profile validator, and sends only
  profile-approved deterministic replies to application terminal queries.
  Query/reply traffic is first-class trace data; no general xterm emulation is
  implied.
- `terminal_baseline` captures lifecycle state from the known initial model;
  `assert_terminal_restored` compares only the applicable, application-owned
  lifecycle state represented by the selected backend. It excludes terminal
  history/visible contents and never promises restoration after `SIGKILL`.
- `PtyTestError` is structured. Every crate-owned timeout/assertion reports the
  scenario, operation/predicate, elapsed monotonic time, latest status,
  dimensions, semantic screen, last event summaries, protocol violation if any,
  and artifact path. Library calls return `Result`; test code decides whether
  to use `?`, `expect`, or an assertion.

Do not add application-specific helpers such as `wait_for_prompt`,
`run_command`, `wait_for_status_row`, or OpenRouter fixtures. Those remain
small adapters in the consuming repository, composed from the public API.

## Internal architecture

Keep implementation-specific modules private and narrow:

```text
src/lib.rs          public types and documentation only
src/config.rs       CommandSpec, Scenario, TestEnv, Size, deadlines
src/unix.rs         PTY creation, spawn, resize, signal, reap, close
src/io.rs           nonblocking read/write, poll loop, event timeline
src/terminal.rs     private backend boundary -> TerminalState/ScreenSnapshot
src/snapshot.rs     stable textual format, comparison, explicit updates
src/protocol.rs     profile scanner, terminal queries, responder, violations
src/artifact.rs     ordered trace, failure bundle, replay metadata
tests/support/      deterministic fixture binary
tests/              crate-level acceptance and invariant tests
```

### Unix spawn and cleanup contract

The Unix implementation uses `openpty` plus a minimal, documented child setup:

1. Open a master/slave pair with the requested non-zero window size. Mark every
   harness descriptor close-on-exec before spawning so concurrent `ptytest`
   cases cannot keep another test's slave open.
2. Prepare the program, arguments, environment, working directory, and a
   close-on-exec error pipe before `fork`.
3. In the child, execute only the reviewed low-level sequence: close master,
   create a session/process group, acquire the slave as the controlling
   terminal, explicitly make that group foreground with `tcsetpgrp`, verify
   `tcgetpgrp`, duplicate the slave to descriptors 0/1/2, close the original
   slave, change directory, and `execve`. On a failure, write a fixed error
   record to the error pipe and `_exit`; do not allocate, lock, format, or
   invoke Rust destructors after `fork`.
4. In the parent, close the slave, set the master nonblocking, read the error
   pipe to distinguish spawn failure from an application exit, retain the
   session-leading PID as the original process-group identity, and begin the
   terminal peer/model at the requested size.

Linux-specific linking details for `openpty` live only in `src/unix.rs`;
macOS uses its native libc entry point. Do not duplicate ioctl numbers or C
layouts throughout the crate.

All master writes handle partial writes, `EINTR`, `EAGAIN`, and `EWOULDBLOCK`
with `poll(POLLOUT)` until the caller's deadline. Reads handle `POLLIN`,
`POLLHUP`, `POLLERR`, `POLLNVAL`, `EINTR`, `EAGAIN`, `EWOULDBLOCK`, EOF, and
the platform `EIO` slave-close convention. A HUP/ERR that arrives with readable
data drains through `EAGAIN` before classifying closure, so final restoration
bytes are retained. PTY closure is not child exit; reaped/observed child status
remains authoritative because a descendant can still hold the slave. Each
chunk is fed unchanged to raw output, the independent model, and the profile
scanner.

`Drop` is a safety net, not a success path. It records artifacts before
destructive cleanup and retains process/session identity until group cleanup is
complete. Before consuming/reaping the leader status, it uses a non-consuming
status observation where the platform supports one, signals the still-verified
original group with `SIGTERM`, polls, escalates once to `SIGKILL`, and only then
reaps the original child. Once that leader has been reaped, it never signals
that numeric group ID again because PID reuse makes it unsafe. This promises
cleanup only for descendants that remain in the original group. Explicit
`wait_for_exit`, `send_eof`, and `hangup` let tests state their intended
lifecycle without pretending a PTY supports half-close.

### Terminal model and snapshots

Use the independent `vt100 0.16.2` model initially. It is already proven in
all four repositories and exposes cells, styles, cursor state, alternate
screen, bracketed-paste mode, and incremental parsing. Keep it behind a private
terminal-backend boundary with an explicit capability record; application tests
depend only on `ScreenSnapshot`/`TerminalState`, never backend types or
formatting. Before exposing a state, verify that the selected backend represents
it faithfully. A gap becomes a focused raw-protocol assertion, a model-layer
augmentation, or a reason to select a stronger backend—not fabricated state.
Record every such gap during implementation. Add parser tests for split CSI,
split UTF-8, wide/combining/zero-width characters and continuation cells,
erasure, resize, alternate screen, and mode changes.

The default snapshot is deliberately readable and stable:

```text
size: columns=40 rows=10
cursor: row=7 column=14 visible=true
modes: alternate-screen=true application-cursor=false bracketed-paste=true

00│ Projects····························
01│
02│ > ghostty
...
09│ q quit·······························
```

Trailing cells are represented consistently with visible-space markers in
diagnostic/snapshot mode; normal `ScreenSnapshot::contains` and row helpers do
not force callers to reason about them. `SnapshotOptions::with_attributes()`
adds sorted, coalesced ranges containing only non-default attributes. Attribute
and Unicode-cell assertions remain available without making every golden noisy.
The default golden remains visible semantics plus selected lifecycle modes; the
separate `TerminalState` query/diagnostic view carries supported model state
needed for lifecycle, mode, wrap, and scroll assertions.

`assert_snapshot(path)` compares this project-owned representation. Ordinary
runs never modify files. `PTYTEST_UPDATE_SNAPSHOTS=1` is the only update path;
it writes the supplied path atomically and reports the change. Missing or
mismatched snapshots show a concise inline diff and create an artifact bundle.
No third-party snapshot framework is needed.

### Terminal peer and lifecycle restoration

`TerminalPeer` is the active side of the terminal protocol, not an emulator.
When the profile admits an application query such as DA, DSR, or CPR, the peer
recognizes it incrementally and writes the deterministic approved reply through
the PTY master. Profiles deny query replies by default. Query detection, reply
generation, terminal-state mutation, and
profile validation remain distinct correctness responsibilities even when they
share decoded controls internally. A query absent from the selected profile is
an immediate structured `UnsupportedTerminalQuery` failure, with its byte
offset and control details, rather than an unanswered child wait.

The independent model starts from a known baseline. Lifecycle assertions compare
only application-responsible state the backend exposes faithfully: applicable
alternate screen, cursor visibility, application cursor/keypad, bracketed
paste, mouse modes, origin/margin modes, scroll region, wrapping, and saved
state. A representative normal exit in every TUI migration must restore its
captured baseline; recoverable Ctrl-C, SIGTERM/error, and panic/unwind paths are
tested where the application claims restoration. `SIGKILL` has no restoration
expectation. Slave-termios inspection may be added as an opt-in diagnostic; it
does not complicate ordinary PTY ownership.

### Failure artifacts and replay

Artifact capture is enabled by default for a `Scenario`; successful cases do
not write anything. A failure creates a unique directory under
`target/ptytest-failures/<sanitized-scenario>-<pid>-<counter>/` containing:

```text
command.txt          program, arguments, cwd, and redacted sensitive values
configuration.txt    size, terminal/profile, deadline, platform
events.jsonl         versioned, ordered replayable event trace
events.log           readable summary of the same event trace
input.bin            exact bytes sent to the PTY
output.bin           exact bytes read from the PTY
screen.ptytest       latest normalized semantic state
terminal-state.txt   supported diagnostic/lifecycle model state
exit-status.txt      last observed/reaped child status
expected.ptytest     expected state for a snapshot mismatch
actual.ptytest       actual state for a snapshot mismatch
diff.txt             snapshot diff where applicable
failure.txt          operation, error, and protocol details
```

`events.jsonl` is canonical: every record has a trace version, monotonically
increasing sequence number, monotonic elapsed time, event kind, and payload.
PTY reads identify their `[offset, length]` range in `output.bin`; writes and
terminal-generated replies identify their range in `input.bin`. Spawn, resize,
signal, hangup, child-status observation, predicate evaluation, query/reply,
and snapshot/checkpoint events occupy the same ordered stream.

Environment values, command arguments, and test-defined values marked secret
are redacted in textual diagnostics. `input.bin` and `output.bin` remain exact
and are therefore intentionally not generically redacted; tests must use fake
credentials whenever possible. Raw buffers have an explicit configurable cap;
reaching it is a visible `TraceLimitExceeded` error, never silent truncation.
Later add `ptytest-replay <failure-bundle>` outside the core test path. It
replays the ordered events—not merely `output.bin`—to reconstruct state with
recorded chunk boundaries and resizes, print exact snapshots, step/dump after
each read, show input/signal/resize events, and jump to an event or byte offset
from a protocol violation. It never re-executes the application.

### Protocol profiles

Protocol validation is a second consumer of raw output, independent of the
semantic parser and terminal responder:

```text
PTY output -> terminal backend -> semantic assertions/snapshots
           -> ProtocolProfile scanner -> compatibility violations/query events
                                      -> TerminalPeer -> approved input replies
```

It is disabled for the initial crate smoke tests and opt-in per scenario until
each consuming project has an audited profile. The scanner reports byte offset,
raw bytes, control family, parsed parameters, and the profile rule that failed.
It must recognize a sequence split across read chunks.

Create `xterm-minimal-v1` by collecting representative traces, inventorying
observed controls, reviewing each required family, documenting its semantic
reason, and then explicitly admitting it to the versioned profile. Traces are
evidence, never automatic authority: an accidental extension is fixed in the
application rather than blessed. Every nontrivial admitted family has a rule,
reason, positive test, and useful negative boundary test; provenance identifies
the repository/scenario, trace span, observed bytes, and review decision. OSC,
DCS, mouse modes, terminal queries, and future xterm extensions remain rejected
unless a project deliberately adds such a rule and responder behavior where
needed. Active profile/version and rule provenance are recorded in
`configuration.txt` and violations. Never label this profile “VT100” merely
because the `vt100` crate can emulate its output.

## Implementation sequence

Each numbered step is a separate, reviewable change. Do not start a later step
until the listed acceptance checks pass. Do not run formatters, linters,
pre-commit hooks, or pushes as part of this work.

### 0. Preserve baselines and approve dependencies

1. Record the current behavior and focused command for each repository before
   edits:

   ```text
   pi-agent: cargo test -p pi-agent-tui --features pty-harness --test pty_streaming
   e:        cargo test --test e2e
   ish:      cargo test --test pty
   tm2:      cargo test --test attached_client_pty
   ```

2. Run each focused command in its own repository and save failures as baseline
   evidence. Inspect worktrees before every edit; preserve unrelated changes.
3. Present the exact `ptytest/Cargo.toml` dependency proposal (`libc` and
   `vt100` only) to the user and wait for approval. If either dependency is not
   approved, revise this plan before implementation; do not substitute a
   larger PTY abstraction without another explicit decision.

Acceptance: the baseline commands and platform/toolchain constraints are
recorded, and dependency authorization is explicit.

### 1. Create the crate and prove the PTY kernel boundary

1. Add the package manifest, public crate docs, license/package metadata as
   directed by the user, and the private module layout above. Gate the crate
   with `#![cfg(unix)]` and emit an intentional compile error on unsupported
   targets rather than offering a partial backend.
2. Implement `Size`, `CommandSpec`, `Scenario`, `TestEnv`, scratch-directory
   ownership, and structured errors first. Unit-test non-zero size validation,
   environment clearing/overrides, path-label sanitization, and parallel
   scratch-root uniqueness. Validate the ASCII and UTF-8 locale policy on both
   CI platforms before Unicode application scenarios rely on it.
3. Add `tests/support/ptytest-fixture.rs`, a deterministic binary with
   line-protocol commands: report TTY ownership, SID/PGID/foreground PGID and
   `TIOCGWINSZ`; acknowledge input; prove a foreground-group `SIGWINCH`;
   enter/leave alternate screen; issue approved/split terminal queries; emit
   split output; and exit with a requested code. It must not depend on a shell
   or arbitrary sleeps.
4. Implement `src/unix.rs` using the spawn contract above. Add acceptance tests
   that prove stdin/stdout/stderr are TTYs, the child has a controlling terminal,
   SID/PGID are correct, `tcgetpgrp(controlling_tty)` equals the child foreground
   PGID, the requested initial `TIOCGWINSZ` is visible, direct exec succeeds,
   and a nonexistent binary reports spawn failure. Prove the same foreground
   group receives terminal-generated resize/interrupt behavior on both OSes.

Acceptance: the fixture passes on macOS and Linux; a rejected spawn leaves no
child; parallel fixture tests do not leak descriptors or scratch state; and
terminal-query replies occur only when their profile enables them.

### 2. Add deterministic I/O, deadlines, resize, and cleanup

1. Implement nonblocking master I/O and the monotonic poll loop. Write the
   versioned ordered event trace for every spawn, read, write, terminal reply,
   resize, signal, hangup, exit observation, predicate outcome, and checkpoint,
   including raw-byte ranges and no timeout resets across partial operations.
2. Implement `send_bytes`, `send_text`, `send_key`, and `send_fragmented`.
   Test full writes, every split boundary of a CSI key sequence, every split
   boundary of a multi-byte UTF-8 character, and `EAGAIN`/partial-write
   progress via the fixture's bounded backpressure mode. Test cursor-key bytes
   before/after application-cursor mode, explicit encoding override, and raw
   input as independent contracts.
3. Implement `resize`, `signal`, `send_eof`, `hangup`, `wait_for_exit`, and
   bounded `finish`/`Drop`. Test the fixture's observed kernel size and
   foreground-group SIGWINCH after very small/wide/tall/rapidly alternating
   sizes, input immediately before/after resize, output-pending resize, and
   return to the original size. Separately test Ctrl-D, termios EOF, real
   hangup, normal exit, SIGINT, forced termination, and a child that spawns an
   ordinary descendant.
4. Make every timeout error snapshot process status, raw byte counts, latest
   events, terminal dimensions/state, and a preliminary artifact bundle before
   cleanup. Exercise `POLLHUP`/`POLLERR` with final readable data, EOF, EIO, and
   invalid-descriptor paths without losing final restoration bytes.

Acceptance: no crate test uses `thread::sleep` for synchronization; all waits
have explicit deadlines; timeout/cleanup tests complete within their own hard
test deadline and leave no fixture process alive.

### 3. Add semantic terminal state and snapshots

1. Wrap `vt100` behind the private capability boundary and expose immutable
   owned `ScreenSnapshot` plus supported `TerminalState` text, row, cursor,
   cell, mode, wrap, scroll, and attribute queries. Keep the parser private and
   record unsupported backend capabilities rather than inferring them.
2. Implement `wait_for_screen` and `wait_for_screen_change` by draining ready
   output, updating terminal state, then evaluating the predicate. Predicate
   descriptions are mandatory so diagnostics explain what timed out.
3. Define and test the textual snapshot grammar, visible-whitespace rendering,
   UTF-8/wide/combining/continuation-cell normalization, default attribute
   elision, compact attribute ranges, comparison diff, explicit update behavior,
   and baseline/restoration comparison.
4. Convert fixture tests from raw-output matching to state predicates and
   snapshots. Retain raw-byte assertions for exact fidelity and lifecycle tests
   for normal exit plus applicable recoverable error/interrupt paths.

Acceptance: replaying one fixture output under every possible chunk boundary
produces byte-for-byte equal `ScreenSnapshot`s. Snapshot comparison failures
show a useful inline diff and write the expected artifact files.

### 4. Audit and enforce protocol profiles

1. Add the incremental control-sequence scanner, query detector/responder, and
   small profile data model. Unit-test every-byte splits, adjacent controls,
   UTF-8 around controls, C0 controls where defined, valid CSI/ESC/private
   modes, invalid/overlong parameters, forbidden OSC/DCS termination variants,
   and incomplete controls at process exit.
2. Run representative initial traces from each application through the scanner
   in reporting mode. Check in a concise per-project inventory with scenario,
   trace span/bytes, semantic reason, and review decision; do not guess
   requirements from terminal library documentation or auto-admit observations.
3. Implement `xterm-minimal-v1`, then enable it for one representative test in
   each migrated repository. Add a negative fixture test for a prohibited
   sequence/query, a positive test for every allowed family, and deterministic
   query/reply trace tests.

Acceptance: a new, unapproved output control sequence fails with the exact
offset and rule; valid current application traces pass without relaxing the
profile to “anything the emulator accepts.”

### 5. Complete diagnostics and contributor documentation

1. Finish artifact serialization, panic-time `Drop` capture, bounded raw-trace
   caps, secret redaction, separate expected/actual/diff snapshots, and the
   complete-bundle event-by-event replay debugger.
2. Write `README.md` sections covering the terminal boundary, the public API,
   hermetic fixtures, semantic predicates, snapshot update command, protocol
   profiles, artifact reading, process cleanup, and macOS/Linux constraints.
3. Document the test-author rule: use screen/predicate barriers; never add a
   delay to make an interaction pass. For a genuinely elapsed-time behavior,
   make the application's clock controllable or assert the domain event rather
   than turning a settle delay into generic harness API.

Acceptance: a new contributor can add and diagnose the fixture example solely
from the README; an intentional timeout identifies its predicate and artifact
directory without rerunning under a debugger.

### 6. Validate the crate on both supported systems

1. Add CI jobs for the pinned toolchain on one macOS and one Linux runner.
   Run the crate unit tests and PTY acceptance tests there; do not treat Linux
   success as macOS evidence.
2. Add a deterministic seeded schedule test to the fixture: bounded sequences
   of input and output fragmentation, resize, signal, hangup, drain, deadline
   exhaustion, and exit. A failure persists its seed and ordered operation trace.
   Promote any found defect to a small named regression test.
3. Add a dedicated output-replay test which splits captured output at
   deterministic boundaries and compares semantic state and profile results.
   Name the second-model canonical-trace corpus as the required post-migration
   hardening step; it is not a legacy-harness migration blocker.

Acceptance: all core semantic snapshots are identical on macOS and Linux; any
legitimate difference is explained before a platform-specific golden is added.
Input fragmentation is an application-decoder invariant; output fragmentation
is a terminal-model/profile-parser invariant. Keep their tests and failures
separate.

## Repository migration plan

Migrate one repository at a time. A migration first adds `ptytest` as a dev
dependency, retains application-specific fixtures and predicates, ports a
representative scenario, then replaces the legacy harness internals. Do not
delete old code or its direct dependencies until the equivalent migrated suite
passes. Run the focused command from Step 0 before and after each migration,
then the repository's normal full test command before moving on.

Every migration records the same auditable transition: baseline focused test;
thin adapter or representative `ptytest` case; focused suite passing; full suite
passing; then legacy harness/dependency removal. Every true TUI retains a
semantic startup barrier, real input, real resize, normal clean exit with
terminal-baseline restoration, and one applicable interrupt/error path. Shell
and job-control software also proves foreground-process-group behavior;
streaming software keeps external fixture barriers; editor tests retain precise
cell/style assertions where they say more than a whole snapshot.

### 7. `tm2`: prove the nested-PTY case first

Owner targets:

- Replace the test-local outer `TmPty` in
  `tests/attached_client_pty.rs` with `ptytest`.
- Preserve `src/pty.rs`; it is production multiplexer code, not obsolete test
  harness code.
- Preserve `tests/support/pty_fixture.rs`; it is an excellent deterministic
  child fixture, then add it to `ptytest` only if a behavior is broadly useful.

Work:

1. Give the test a hermetic `TM_SOCKET` path under the `ptytest` scratch root
   rather than a global `/tmp` name.
2. Spawn the attached `tm` client through `PtyTest`, retain the existing
   readiness/input/resize/detach barriers as screen predicates, and assert the
   fixture's `RESIZE_ACK:10x50` after a real outer-PTY resize.
3. Add snapshots for ready and resized attached-client screens only if they
   clarify a user-visible invariant; marker barriers remain appropriate for the
   fixture protocol.
4. Add the project's first `xterm-minimal-v1` reporting/profile test and
   preserve attach-client exit behavior and terminal restoration after detach.

Acceptance: the focused nested-PTY test passes repeatedly in parallel, leaves
no socket/server/client process, and still proves resize propagation rather
than merely an outer ioctl; its adapter/removal transition has passed the common
lifecycle matrix.

### 8. `pi-agent-tui`: migrate the streaming real-binary test

Owner targets:

- Replace `tests/pty_streaming.rs::PtyApp` with `ptytest`.
- Keep the offline OpenRouter fixture; it is application-specific network
  control, not part of `ptytest`.
- Remove direct `portable-pty` and `vt100` dev dependencies only after the
  migrated test builds under `pty-harness`.

Work:

1. Build `CommandSpec` from `CARGO_BIN_EXE_pi-agent`, retain explicit provider,
   model, endpoint, and offline key arguments/environment, and place every
   other state directory in `TestEnv::hermetic()`.
2. Replace text polling with named screen predicates for model readiness, the
   first released streaming token, completion/idle, and clean Ctrl-C exit.
   Keep the fixture channel as the streaming-settlement barrier.
3. Add one narrow resize/lifecycle regression that observes `TerminalGuard`'s
   alternate screen, cursor/bracketed-paste modes, and restoration on normal
   exit through semantic state/protocol trace rather than internal calls; retain
   one applicable Ctrl-C or error-exit restoration/cleanup path.

Acceptance: the current “first token before settlement” guarantee remains
covered by the real binary, with no reader thread, raw `vt100` parser, or
sleeping exit loop in the test file, and the common lifecycle matrix passes.

### 9. `e`: replace internals behind its established E2E API

Owner target: refactor `tests/e2e/harness.rs::TestEditor` into a thin
application adapter over `PtyTest`; do not rewrite hundreds of user-facing
tests in one patch.

Work:

1. Map `new`, `with_size`, fixture/home setup, raw/text/special-key input,
   mouse bytes, screen rows, cursor, and color/inverse assertions onto the
   shared API. Keep editor-specific commands, status-row knowledge, and PGO
   recording policy in the adapter.
2. Replace its handwritten `openpty`, FFI C layouts, pre-exec setup, reader
   thread, timeout/quiescence logic, and process-group cleanup. Preserve its
   important test fixtures and parallel isolated state.
3. Convert the broad startup heuristic to an editor-specific semantic
   predicate, then use state predicates for resize and redraw tests. Do not put
   “wait for the e status row” into `ptytest`.
4. Port representative startup, Unicode editing, narrow resize/repaint,
   bracketed paste, style, save, and clean-quit tests to semantic snapshots;
   leave existing focused row/cell assertions where they better state the
   behavior. Assert terminal-baseline restoration on normal quit and an
   applicable recoverable interrupt/error path.
5. Keep recording output byte-faithful if it remains supported; any cast/GIF
   conversion belongs in `e`, not the core harness.

Acceptance: `cargo test --test e2e` passes with the same coverage categories;
all old harness-only unsafe/platform code is deleted; normal E2E tests do not
depend on the PGO recording mode; the common lifecycle matrix passes.

### 10. `ish`: replace the private shell runner, then remove timing crutches

Owner target: retain the `PtyShell` name only as a thin shell-specific adapter
in `tests/pty.rs`, backed by `PtyTest`.

Work:

1. Replace binary discovery, temporary HOME allocation, direct fork/openpty,
   descriptor handling, poll loops, raw ANSI stripping, local `Screen`, and
   process teardown with crate facilities. Keep ish's setup helpers for files,
   history, config, envrc, and the semantic “prompt after command newline”
   barrier.
2. Preserve raw keyboard and byte helpers where names improve shell test
   readability, but implement them with `send_key`/`send_bytes`.
3. Convert `wait_for_screen` to retain one incremental shared terminal model;
   do not replay a lossy string into a fresh parser on every predicate.
4. Replace UI settle sleeps one scenario at a time with a screen/cursor/prompt
   predicate. For tests whose actual contract is filesystem timestamp
   granularity, introduce a narrow application clock/metadata test seam or a
   deterministic fixture—do not disguise the sleep as a generic PTY wait.
5. Port history search, completion grid, resize re-anchor, job control,
   bracketed paste, Ctrl-C, Ctrl-D, and clean exit as representative semantic
   snapshots or focused state assertions, including foreground-process-group
   behavior and terminal restoration for normal/recoverable exits. Add protocol
   validation only after their observed sequence inventory is reviewed.

Acceptance: `cargo test --test pty` passes without private PTY/process/parser
implementation code; all remaining time delays name a domain constraint and
are documented beside that test; the common lifecycle matrix passes.

### 11. Finish cross-repository standardization

1. Align all consumers on a compatible `ptytest` version and `vt100` through
   the crate; remove now-unused direct test-only parser/PTY dependencies.
2. Add the same contributor-facing `PTYTEST_UPDATE_SNAPSHOTS=1` convention and
   artifact-location documentation to each repository. Snapshot files live next
   to the owning integration scenario, not in a global shared directory.
3. Enable an audited protocol profile for at least one representative PTY test
   in every repository, then expand only after each project's own traces pass.
4. Add one bounded invariant-oriented seeded schedule test for a selected real
   interactive binary where it is cheap: key/input, fragmented input, resize,
   drain, SIGINT, hangup, and normal quit. It asserts no panic/deadlock/leak,
   bounded completion, valid enabled profile/model state, and restoration where
   the scenario exits normally; persist seeds and promote defects.
5. Document the UTF-8 test-environment policy, replayable failure bundles, and
   `PTYTEST_UPDATE_SNAPSHOTS=1` in every consumer.
6. Re-run all Step 0 focus tests under normal Cargo parallelism and
   `-- --test-threads=1`; run the full suites and both OS CI jobs. Report any
   non-PTY failures separately rather than hiding them with harness changes.

Acceptance: all four repositories exercise the same public crate at their
terminal boundary, have no duplicate PTY spawn/poll/cleanup implementation in
their test trees, retain their application-specific fixtures and behavior tests,
and meet the common lifecycle matrix.

### 12. Differential oracle hardening

Collect a canonical selected corpus of recorded traces: startup,
alternate-screen lifecycle, cursor movement, wide/combining Unicode, erasure,
resize/reflow-sensitive behavior where applicable, bracketed-paste/mode
transitions, and representative application workflows. In a pinned nightly or
dedicated CI job, replay each trace and recorded operation schedule through the
primary backend and a second mature independent terminal model, then compare
capability-normalized state. Preserve both states, backend versions, and the
offending event/byte span on disagreement. This is required end-state
hardening, not a prerequisite for deleting legacy harnesses.

## Definition of done

This work is complete only when all of the following are true:

- Real compiled binaries execute under correctly configured macOS/Linux PTYs;
  fixture evidence proves TTY ownership, controlling-terminal state, SID, PGID,
  foreground PGID, resize delivery, tty-generated signals, hangup, exit, and
  bounded cleanup.
- All PTY I/O is nonblocking/deadline-bounded and correct across partial
  operations, poll/hangup races, EOF/EIO, and final restoration bytes. Logical
  key helpers respect represented terminal input modes; `send_bytes` remains
  exact.
- Terminal queries receive deterministic, profile-approved replies or fail
  explicitly. The incremental independent model is hidden behind project-owned
  semantic/model state and never fabricates an unsupported capability.
- Semantic snapshots are stable, readable, explicitly updated, and independent
  of repaint strategy. Representative TUIs prove terminal lifecycle restoration
  on normal exits and applicable recoverable failures, never demanding it after
  `SIGKILL`.
- Raw exact input/output and a versioned ordered event trace are retained for
  failure forensics. Bundles include current/expected/actual semantic state,
  actionable diffs, status, dimensions, recent events, and redacted textual
  diagnostics; raw traces are explicitly documented as potentially sensitive.
- `ptytest-replay` deterministically reconstructs and inspects terminal state
  across recorded event boundaries without re-executing the application.
- Protocol profiles are normative, provenance-backed compatibility contracts,
  not inferred from `TERM` or automatically generated from observed traces.
  Input fragmentation and output-parser fragmentation are independently tested.
- Fixture and selected real-binary schedule stress persist seeds/operation
  traces and promote meaningful failures to named regressions. The explicit
  UTF-8 locale policy, all Layer A-E checks, and every migration transition gate
  pass on both platforms under normal and single-threaded scheduling.
- The representative workflows have been ported and only then has each legacy
  test-harness implementation been removed. Ordinary noninteractive CLI tests
  remain ordinary pipe/subprocess tests; no normal case leaks the child or an
  ordinary descendant.
- A selected canonical corpus is cross-checked in nightly/dedicated CI against
  a second independent terminal model, with both normalized states retained on
  disagreement. Core semantic goldens remain platform-identical unless a
  reviewed OS distinction genuinely requires otherwise.

The enduring architectural invariant is:

```text
real kernel PTY
+ real compiled application
+ independent semantic terminal model
+ audited terminal protocol contract
+ deterministic terminal peer behavior
+ replayable forensic trace
```
