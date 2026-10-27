# ptytest

`ptytest` is a Unix-only (macOS and Linux) Rust test-support crate for testing
compiled interactive programs through a real kernel PTY. It sends actual
terminal input, resizes the PTY, and exposes an independent semantic screen
model for assertions.

The package is published as `laputa-ptytest`; the library target is named
`ptytest`:

```toml
[dev-dependencies]
laputa-ptytest = "0.3"
```

## Use in a test

```rust
use ptytest::{CommandSpec, ExitStatus, Key, PtyTest, Scenario, Size, TestEnv};
use std::time::Duration;

#[test]
fn test_startup() -> Result<(), ptytest::PtyTestError> {
    let scenario = Scenario::new("startup")?
        .command(CommandSpec::new("/path/to/example"))
        .size(Size::new(100, 24)?)
        .environment(TestEnv::hermetic()?);
    let mut terminal = PtyTest::spawn(scenario)?;

    terminal.wait_for_screen(terminal.deadline(Duration::from_secs(3)), "project list", |screen| {
        screen.contains("Projects")
    })?;
    terminal.send_key(terminal.deadline(Duration::from_secs(3)), Key::Ctrl('c'))?;
    let status = terminal.wait_for_exit(terminal.deadline(Duration::from_secs(3)))?;
    assert_eq!(status, ExitStatus::Code(0));
    let status = terminal.finish(terminal.deadline(Duration::from_secs(3)))?;
    assert_eq!(status, ExitStatus::Code(0));
    Ok(())
}
```

Operations that may wait take an explicit `Deadline`. A timeout or failed
assertion writes a failure bundle under `target/ptytest-failures/`, including
the screen, event trace, and captured input/output. Inspect it offline with:

```sh
cargo run --locked --bin ptytest-replay -- target/ptytest-failures/<bundle>
```

Snapshot files update only when `PTYTEST_UPDATE_SNAPSHOTS=1` is set. Text
diagnostics redact `secret_env` values, while raw input/output files retain
unredacted captured bytes up to the trace limit; use fake credentials. See
[API details](docs/api.md) and [backend limits](docs/backend-capabilities.md).

Source and issue tracker: <https://github.com/laputa-systems/ptytest>.

## License

MIT
