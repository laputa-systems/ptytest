# ptytest

`ptytest` is a Unix-only (macOS and Linux) Rust test-support crate. It tests
a compiled interactive program at its real terminal boundary: it owns a kernel
PTY, executes the supplied binary directly (never through a shell), writes
real input bytes, resizes with `TIOCSWINSZ`, and keeps an independent `vt100`
screen model for assertions.

The package is published as `laputa-ptytest`; the library target is named
`ptytest`:

```toml
[dev-dependencies]
laputa-ptytest = "0.2"
```

## Usage

```rust
use ptytest::{CommandSpec, Key, PtyTest, Scenario, Size, TestEnv};
use std::time::Duration;

let scenario = Scenario::new("startup")?
    .command(CommandSpec::new("/path/to/example"))
    .size(Size::new(100, 24)?)
    .environment(TestEnv::hermetic()?);
let mut terminal = PtyTest::spawn(scenario)?;

terminal.wait_for_screen(terminal.deadline(Duration::from_secs(3)), "project list", |screen| {
    screen.contains("Projects")
})?;
terminal.send_key(terminal.deadline(Duration::from_secs(3)), Key::Ctrl('c'))?;
terminal.wait_for_exit(terminal.deadline(Duration::from_secs(3)))?;
terminal.finish(terminal.deadline(Duration::from_secs(3)))?;
```

Every operation that may wait takes an explicit `Deadline`. On a timeout or
failed assertion, a failure bundle (raw input/output, event trace, screen) is
written to `target/ptytest-failures/` and can be replayed offline with
`cargo run --bin ptytest-replay -- target/ptytest-failures/<bundle>`.

`AGENTS.md` is the maintainer guide, and `docs/` has the protocol profiles,
backend capabilities, and baselines.

## Make targets

- `make test` runs `cargo test --locked`.
- `make lint` runs `cargo clippy --fix`.
- `make publish` runs `tools/bump_minor_release.sh`.

## License

MIT
