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
test code -> Unix PTY master -> binary stdin/stdout/stderr -> terminal bytes
              ^                                              |
              |                                              v
       input, resize, signals                       raw trace + independent
                                                     terminal state
```

The test oracle is normalized terminal state. Raw bytes remain available for
protocol assertions and failure diagnosis, but are never the primary visual
golden.

The finished crate must make these guarantees:

- Every child has a PTY-backed stdin, stdout, and stderr; a controlling terminal;
  a fresh session/process group; termios; and an initial kernel window size.
- Tests send actual bytes through the master and resize through
  `TIOCSWINSZ`; they never invoke application input or resize internals.
- Every operation that may wait uses a monotonic, caller-supplied deadline.
  Sleeps are not a synchronization primitive in `ptytest`.
- Output is interpreted incrementally, so arbitrary output chunk boundaries,
  split escape sequences, and split UTF-8 are valid.
- Timeouts and crate-owned assertions retain a readable screen, raw input and
  output, event history, configuration, and process state. Dropping a live
  harness during an unwinding panic writes the same bundle before cleanup.
- Cleanup is bounded and targets the child process group; it cannot leave a
  normal child or its ordinary descendants behind.

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
`xterm-minimal-v1`. The exact allowed sequences are discovered from recorded
representative traces before enforcement. A profile must describe the real
contract; it must not be inferred from `TERM`.

## Scope and non-goals

In scope:

- Direct execution of a supplied binary under a macOS/Linux PTY.
- Input bytes, keyboard convenience encodings, resize, explicit signals, exit
  status, terminal state, snapshots, protocol validation, and artifacts.
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
```

### Types and invariants

- `Size { columns, rows }` is non-zero at construction. It uses terminal
  ordering—columns first—and converts to the kernel's rows/columns only in the
  Unix boundary.
- `CommandSpec` owns an executable path, OS-string arguments, current
  directory, and explicit environment additions/removals. It executes that
  path with `execve` (no shell and no implicit `PATH` lookup); it never accepts
  a shell command line. Owning it makes failure artifacts reproducible without
  relying on `std::process::Command` internals.
- `Scenario` has a caller-provided, stable label. It determines the artifact
  directory and makes snapshot failures identifiable. Labels are sanitized for
  paths; an invalid/empty label is an error, not a fallback name.
- `TestEnv::hermetic()` clears inherited application environment and creates a
  unique scratch root containing `home`, `config`, `data`, `cache`, `state`,
  and `tmp`. It sets `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`,
  `XDG_CACHE_HOME`, `XDG_STATE_HOME`, `TMPDIR`, `TERM=xterm-256color`,
  `PATH=/usr/bin:/bin`, `LC_ALL=C`, and `TZ=UTC`. Callers may explicitly
  override or add variables; inherited environment is opt-in. The scratch root
  and a fixture subdirectory are exposed for test setup.
- `PtyTest` exclusively owns the master, child PID/process group, incremental
  terminal model, raw input/output buffers, event log, configuration, and
  artifact policy. It is neither `Clone` nor `Sync`.
- `send_bytes`, `send_text`, `send_key`, and `send_fragmented` write the real
  byte stream. `Key` covers unambiguous common xterm byte encodings
  (characters, Enter, Escape, Tab, Backspace, arrows, and Ctrl characters).
  Application-cursor mode, mouse, bracketed paste, and any ambiguity retain a
  `send_bytes` escape hatch; the crate does not pretend that all terminal
  keyboards share one encoding.
- `resize` changes the master with `TIOCSWINSZ`; it does not call application
  code or separately synthesize `SIGWINCH`. The controlling terminal's normal
  foreground-process-group delivery is the behavior under test. `signal`
  explicitly targets the child process group only for lifecycle tests.
- `Deadline` is built from `Instant`, not wall-clock time. `wait_for_screen`,
  `wait_for_screen_change`, `drain`, and `wait_for_exit` all require one.
  There is no unbounded `read`, `wait`, or `sleep` API.
- `ExitStatus` distinguishes a normal code, signal termination, and an
  unreaped/running child. `wait_for_exit` returns the real status and never
  silently asserts success; tests assert their expected status.
- `ScreenSnapshot` is a project-owned copy of semantic state, not a public
  alias for `vt100` types. It includes size, visible rows, cursor position and
  visibility, alternate-screen and relevant input modes, row-wrap state, and
  normalized cells/attributes when requested. It is safe to retain after the
  next read.
- `PtyTestError` is structured. In particular, a timeout records the operation
  description, elapsed time, latest status, latest semantic screen, and an
  artifact path. Library calls return `Result`; test code decides whether to
  use `?`, `expect`, or an assertion.

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
src/terminal.rs     vt100 adapter -> owned ScreenSnapshot
src/snapshot.rs     stable textual format, comparison, explicit updates
src/protocol.rs     profile scanner and violations (introduced later)
src/artifact.rs     failure bundle and replayable trace metadata
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
   create a session, acquire the slave as the controlling terminal, duplicate
   it to descriptors 0/1/2, close the original slave, change directory, and
   `execve`. On a failure, write a fixed error record to the error pipe and
   `_exit`; do not allocate, lock, format, or invoke Rust destructors after
   `fork`.
4. In the parent, close the slave, set the master nonblocking, read the error
   pipe to distinguish spawn failure from an application exit, retain the
   session-leading PID, and begin the terminal model at the requested size.

Linux-specific linking details for `openpty` live only in `src/unix.rs`;
macOS uses its native libc entry point. Do not duplicate ioctl numbers or C
layouts throughout the crate.

All master writes handle partial writes, `EINTR`, `EAGAIN`, and `EWOULDBLOCK`
with `poll(POLLOUT)` until the caller's deadline. All reads drain ready bytes,
handle `EINTR`, and treat PTY EOF and the macOS/Linux `EIO` close convention as
peer closure only after preserving bytes already read. Each received chunk is
fed unchanged to both the raw trace and terminal parser.

`Drop` is a safety net, not a success path: it closes input, sends `SIGTERM` to
the verified child process group, polls/reaps until a short bounded cleanup
deadline, escalates once to `SIGKILL`, and reaps the original child. Explicit
`close_input` and `finish` make ordinary tests able to assert their intended
lifecycle. The implementation must never send a group signal after the child
has been reaped or a PID has become ambiguous.

### Terminal model and snapshots

Use the independent `vt100 0.16.2` model initially. It is already proven in
all four repositories and exposes cells, styles, cursor state, alternate
screen, bracketed-paste mode, and incremental parsing. Keep it behind
`ScreenSnapshot`; application tests must not depend on its formatting or type
layout. Add parser-unit tests for split CSI, split UTF-8, wide characters,
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

`assert_snapshot(path)` compares this project-owned representation. Ordinary
runs never modify files. `PTYTEST_UPDATE_SNAPSHOTS=1` is the only update path;
it writes the supplied path atomically and reports the change. Missing or
mismatched snapshots show a concise inline diff and create an artifact bundle.
No third-party snapshot framework is needed.

### Failure artifacts and replay

Artifact capture is enabled by default for a `Scenario`; successful cases do
not write anything. A failure creates a unique directory under
`target/ptytest-failures/<sanitized-scenario>-<pid>-<counter>/` containing:

```text
command.txt          exact program, arguments, cwd, and redacted environment
configuration.txt    size, terminal/profile, deadline, platform
events.log           monotonic event timeline
input.bin            exact bytes sent to the PTY
output.bin           exact bytes read from the PTY
screen.ptytest       latest normalized semantic state
exit-status.txt      last observed/reaped child status
failure.txt          operation, error, and snapshot/protocol diff if relevant
```

Environment values marked secret are redacted. Raw buffers have an explicit
configurable cap; reaching it is a visible `TraceLimitExceeded` error, never
silent truncation. Later add a tiny `ptytest-replay` example/binary that feeds
`output.bin` through the same terminal adapter and prints a snapshot; it must
not be part of the core test path.

### Protocol profiles

Protocol validation is a second consumer of raw output, independent of the
semantic parser:

```text
PTY output -> vt100 terminal model -> semantic assertions/snapshots
           -> ProtocolProfile scanner -> compatibility violations
```

It is disabled for the initial crate smoke tests and opt-in per scenario until
each consuming project has an audited profile. The scanner reports byte offset,
raw bytes, control family, parsed parameters, and the profile rule that failed.
It must recognize a sequence split across read chunks.

Create `xterm-minimal-v1` from real traces. It will contain only observed,
documented C0 controls, ESC commands, cursor/erase CSI, SGR, and required DEC
private modes (for example cursor visibility, alternate screen, and bracketed
paste) plus each explicitly approved extension. OSC, DCS, mouse modes, and
future xterm extensions remain rejected unless a project deliberately adds a
rule with a test and a reason. Never label this profile “VT100” merely because
the `vt100` crate can emulate its output.

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
   scratch-root uniqueness.
3. Add `tests/support/ptytest-fixture.rs`, a deterministic binary with
   line-protocol commands: report `isatty`, report `TIOCGWINSZ`, acknowledge
   input, wait for/acknowledge `SIGWINCH`, enter/leave alternate screen, emit
   split output, and exit with a requested code. It must not depend on a shell
   or arbitrary sleeps.
4. Implement `src/unix.rs` using the spawn contract above. Add acceptance tests
   that prove the child sees three TTY stdio streams, one controlling terminal,
   the requested initial size, a distinct session/process group, direct exec,
   and a reported spawn error for a nonexistent binary.

Acceptance: the fixture passes on macOS and Linux; a rejected spawn leaves no
child; parallel fixture tests do not leak descriptors or scratch state.

### 2. Add deterministic I/O, deadlines, resize, and cleanup

1. Implement nonblocking master I/O and the monotonic poll loop. Event-log
   every spawn, read, write, resize, signal, exit observation, and predicate
   outcome with elapsed monotonic time.
2. Implement `send_bytes`, `send_text`, `send_key`, and `send_fragmented`.
   Test full writes, every split boundary of a CSI key sequence, every split
   boundary of a multi-byte UTF-8 character, and `EAGAIN`/partial-write
   progress via the fixture's bounded backpressure mode.
3. Implement `resize`, `signal`, `close_input`, `wait_for_exit`, and bounded
   `finish`/`Drop`. Test the fixture's observed kernel size and SIGWINCH after
   repeated resize, resize followed immediately by input, EOF, normal exit,
   SIGINT, forced termination, and a child that spawns an ordinary descendant.
4. Make every timeout error snapshot process status, raw byte counts, latest
   events, and a preliminary artifact bundle before cleanup.

Acceptance: no crate test uses `thread::sleep` for synchronization; all waits
have explicit deadlines; timeout/cleanup tests complete within their own hard
test deadline and leave no fixture process alive.

### 3. Add semantic terminal state and snapshots

1. Wrap `vt100` incrementally and expose immutable owned `ScreenSnapshot`
   values plus text, row, cursor, cell, mode, and attribute queries. Keep the
   parser private.
2. Implement `wait_for_screen` and `wait_for_screen_change` by draining ready
   output, updating terminal state, then evaluating the predicate. Predicate
   descriptions are mandatory so diagnostics explain what timed out.
3. Define and test the textual snapshot grammar, visible-whitespace rendering,
   unicode/wide-cell normalization, default attribute elision, compact
   attribute ranges, comparison diff, and explicit update behavior.
4. Convert fixture tests from raw-output matching to state predicates and
   snapshots. Retain at least one raw-byte assertion to prove artifacts have
   byte fidelity.

Acceptance: replaying one fixture output under every possible chunk boundary
produces byte-for-byte equal `ScreenSnapshot`s. Snapshot comparison failures
show a useful inline diff and write the expected artifact files.

### 4. Audit and enforce protocol profiles

1. Add the incremental control-sequence scanner and a small profile data model.
   Unit-test plain text, C0 controls, valid CSI/ESC, private modes, invalid
   parameters, forbidden OSC/DCS, and all chunk boundaries.
2. Run representative initial traces from each application through the scanner
   in reporting mode. Check in a concise per-project sequence inventory with
   source scenario and rationale; do not guess requirements from terminal
   library documentation.
3. Implement `xterm-minimal-v1`, then enable it for one representative test in
   each migrated repository. Add a negative fixture test for a prohibited
   sequence and a positive fixture test for every allowed family.

Acceptance: a new, unapproved output control sequence fails with the exact
offset and rule; valid current application traces pass without relaxing the
profile to “anything the emulator accepts.”

### 5. Complete diagnostics and contributor documentation

1. Finish artifact serialization, panic-time `Drop` capture, bounded raw-trace
   caps, secret redaction, and the replay helper.
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
   of input fragmentation, resize, signal, close-input, drain, and exit. A
   failure prints/persists the seed and generated operations. Promote any found
   defect to a small named regression test.
3. Add a dedicated output-replay test which splits captured output at
   deterministic boundaries and compares semantic state. Differential testing
   against a second terminal model is an optional nightly follow-up, not a
   migration blocker.

Acceptance: all core semantic snapshots are identical on macOS and Linux; any
legitimate difference is explained before a platform-specific golden is added.

## Repository migration plan

Migrate one repository at a time. A migration first adds `ptytest` as a dev
dependency, retains application-specific fixtures and predicates, ports a
representative scenario, then replaces the legacy harness internals. Do not
delete old code or its direct dependencies until the equivalent migrated suite
passes. Run the focused command from Step 0 before and after each migration,
then the repository's normal full test command before moving on.

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
   preserve attach-client exit behavior.

Acceptance: the focused nested-PTY test passes repeatedly in parallel, leaves
no socket/server/client process, and still proves resize propagation rather
than merely an outer ioctl.

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
   exit through semantic state/protocol trace rather than internal calls.

Acceptance: the current “first token before settlement” guarantee remains
covered by the real binary, with no reader thread, raw `vt100` parser, or
sleeping exit loop in the test file.

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
   behavior.
5. Keep recording output byte-faithful if it remains supported; any cast/GIF
   conversion belongs in `e`, not the core harness.

Acceptance: `cargo test --test e2e` passes with the same coverage categories;
all old harness-only unsafe/platform code is deleted; normal E2E tests do not
depend on the PGO recording mode.

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
   snapshots or focused state assertions. Add protocol validation only after
   their observed sequence inventory is reviewed.

Acceptance: `cargo test --test pty` passes without private PTY/process/parser
implementation code; all remaining time delays name a domain constraint and
are documented beside that test.

### 11. Finish cross-repository standardization

1. Align all consumers on a compatible `ptytest` version and `vt100` through
   the crate; remove now-unused direct test-only parser/PTY dependencies.
2. Add the same contributor-facing `PTYTEST_UPDATE_SNAPSHOTS=1` convention and
   artifact-location documentation to each repository. Snapshot files live next
   to the owning integration scenario, not in a global shared directory.
3. Enable an audited protocol profile for at least one representative PTY test
   in every repository, then expand only after each project's own traces pass.
4. Re-run all Step 0 focus tests under normal Cargo parallelism and
   `-- --test-threads=1`; run the full suites and both OS CI jobs. Report any
   non-PTY failures separately rather than hiding them with harness changes.

Acceptance: all four repositories exercise the same public crate at their
terminal boundary, have no duplicate PTY spawn/poll/cleanup implementation in
their test trees, and retain their application-specific fixtures and behavior
tests.

## Definition of done

This work is complete only when all of the following are true:

- `ptytest` runs actual binaries under a correctly configured macOS/Linux PTY
  and has fixture evidence for TTY/session/resize/signal/cleanup behavior.
- The public API returns structured errors, requires deadlines, parses output
  incrementally into independent semantic state, and supports readable,
  explicitly updated snapshots.
- Failure artifacts contain exact input/output, event history, configuration,
  terminal state, status, and actionable timeout/protocol/snapshot errors.
- Compatibility validation uses an audited named xterm-minimal profile rather
  than an inaccurate VT100 label or a raw ANSI transcript golden.
- The representative workflows above have been ported, then each legacy
  test-harness implementation has been removed only after its replacement
  passes.
- The crate and migrated suites pass on macOS and Linux, under both normal and
  single-threaded test scheduling, without arbitrary synchronization sleeps,
  leaked children, fixed global resources, or undeclared dependencies.
