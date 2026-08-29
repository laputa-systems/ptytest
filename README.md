# ptytest

`ptytest` is a Unix-only Rust test-support crate for testing a compiled
interactive program at its real terminal boundary. It owns a kernel PTY,
executes the supplied binary directly (never through a shell), writes real
input bytes, resizes with `TIOCSWINSZ`, and maintains an independent `vt100`
semantic model for assertions.

It supports macOS and Linux. Windows, browser/GUI automation, shell command
parsing, async runtimes, and terminal-emulator rendering are intentionally out
of scope.

## Package and import names

The package is published as `laputa-ptytest`, while its library target is named
`ptytest`. Add the package name to a consumer's `Cargo.toml`, then import the
library by its shorter crate name:

```toml
[dev-dependencies]
laputa-ptytest = "0.1"
```

```rust
use ptytest::PtyTest;
```

## Basic test

```rust,no_run
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

Every potentially waiting operation accepts a monotonic `Deadline`. Use a
screen predicate or a fixture protocol barrier to synchronize an interaction;
do not add a delay to make a test pass. If a behavior genuinely depends on
elapsed time, control the application's clock or assert its domain event.

## Hermetic fixtures

`TestEnv::hermetic()` allocates a unique scratch root with `home`, XDG config,
data, cache, state, temporary, and `fixtures` directories. It clears inherited
application environment, sets `TERM=xterm-256color`, `PATH=/usr/bin:/bin`,
`LANG=C`, `LC_ALL=C`, and `TZ=UTC`. Use `TestEnv::hermetic_utf8(locale)` only
with a locale verified on both supported CI platforms. `secret_env` redacts an
environment value from text artifacts; raw input/output bytes remain exact and
may contain secrets, so fixtures should use fake credentials.

The crate's `ptytest-fixture` binary demonstrates ownership, window-size,
resize signal, terminal query, input fragmentation, EOF, hangup, and process
group contracts without a shell or settle sleeps.

`C.UTF-8` is the portable Unicode test locale. `hermetic_utf8("C.UTF-8")`
checks the host's locale inventory first (including Darwin's `C.UTF-8` and
glibc's equivalent `C.utf8` spelling), then sets `LANG` and `LC_ALL` together.

## Semantic assertions and snapshots

`ScreenSnapshot` is an owned semantic view, independent of paint chunking. It
provides visible row text, cells and attributes, cursor state, and represented
terminal modes. `TerminalState` intentionally contains only state the backend
can report faithfully; it never invents unrepresented modes.

The initial model deliberately does not claim soft-wrap reflow after a resize;
that boundary and the upgrade path are documented in [terminal backend
capabilities](docs/backend-capabilities.md).

`assert_snapshot(path)` compares a stable `.ptytest` text representation.
Normal test runs never modify snapshots. Update an existing or missing golden
explicitly with:

```text
PTYTEST_UPDATE_SNAPSHOTS=1 cargo test --test your_terminal_test
```

Keep a snapshot beside its owning integration scenario. Prefer small focused
cell/style assertions where they say more than a whole-screen golden.

## Protocol profiles

Profiles validate raw output separately from the semantic parser and control
which terminal queries receive replies. The default profile performs no output
validation and replies to no queries. `ProtocolProfile::xterm_minimal_v1()`
admits only the reviewed controls in
[the profile policy](docs/protocol-profiles.md); unlisted OSC, DCS, private
modes, and queries fail at their output offset.

The profile is a normative compatibility contract, not a consequence of
`TERM` or a list automatically inferred from traces. A consuming project first
collects and reviews its own trace inventory, then enables an audited profile
for one representative scenario before expanding coverage.

## Failures, cleanup, and replay

Crate-owned timeouts and assertions write a unique bundle under
`target/ptytest-failures/`. It includes command/configuration diagnostics,
redacted text environment values, ordered `events.jsonl`, readable events,
exact `input.bin`/`output.bin`, screen and terminal state, exit status, and
snapshot expected/actual/diff files when applicable. Raw buffers are capped by
the scenario and overflow visibly with `TraceLimitExceeded`. A timeout also
records its requested duration when available, elapsed time since the supplied
deadline was created, last observed child status, dimensions, byte counts, and
event count. Raw byte files
are intentionally exact and can contain test secrets.

Replay a bundle without starting the original program:

```text
cargo run --bin ptytest-replay -- target/ptytest-failures/<bundle>
```

Add `--dump-events` to print the validated timeline, `--step` for a semantic
snapshot after each read/resize, `--event N` to stop after event `N`, or
`--byte O` to stop after output byte `O`. Replay validates contiguous raw-byte
ranges and recorded query/reply traffic; it never starts the original program.

`PtyTest::finish` targets the original process group, escalates from `SIGTERM`
to `SIGKILL` within the supplied deadline, and then reaps the original child.
It retains an observed zombie session leader until group cleanup completes, so
ordinary descendants that remain in the original group are still targeted. It
deliberately does not promise to kill a descendant that daemonizes into another
session. Dropping a live harness records a failure bundle before bounded
cleanup.

## Validation evidence

The pinned toolchain, clean consumer baselines, dependency authorization, and
cross-platform validation policy are recorded in [the baseline record](docs/baselines.md).
