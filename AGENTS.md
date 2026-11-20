# ptytest

`ptytest` is a Unix-only Rust test-support crate for exercising interactive programs through a real PTY.

Build: `cargo build --locked --all-targets` · Test: `make test` · Check: `make lint`.

## Verification

- Test tiers: Edit uses a named inline test (`cargo test --locked <name>`); Change uses the affected `io_regressions`, `pty_boundary`, or `replay_cli` integration target; Scenario uses the terminal differential corpus; the explicitly requested full acceptance is `make test`. Individual warm costs are unmeasured; the fresh whole-suite reference is below.
- `make test` and `make lint` are the CI entry points on Linux arm64 and macOS. CI also runs the single-threaded suite. A fresh macOS arm64 target at `3fd48af` passed in 12.26s under host load. The first warm run exposed the deadline equality failure tracked in [#13](https://github.com/laputa-systems/ptytest/pull/13).
- `make coverage` runs the suite with cargo-llvm-cov 0.9.1 and the pinned toolchain's LLVM tools. It writes owned source branch coverage to `.work/coverage/summary.json`.
- Real PTY, direct spawn, process cleanup, resize and signals: `tests/pty_boundary.rs`. Deadlines, quiescence and bounded reads and writes: `tests/io_regressions.rs`.
- Screen cells, modes, split terminal sequences and approved replies: `src/terminal.rs`, `src/protocol.rs` and `tests/pty_boundary.rs`. The scheduled differential corpus checks shared behavior against the independent terminal backend.
- Failure bundles, event JSONL and replay: `tests/pty_boundary.rs` and `src/replay.rs`. Replay CLI help and exit success: `tests/replay_cli.rs`.
- Consumer tests use the crate in e, fz, ish, lz, soulseek-rs, tm and xsh. See [published-pin and historical checks](docs/baselines.md).
- No standalone throughput workload. Consumer PTY cases and the bounded I/O regressions check the observable waits and terminal round trips.

## Invariants

- `CommandSpec` executes the caller's program directly; it never parses a shell command or searches `PATH`.
- `Size` is non-zero and ordered columns then rows; kernel row/column conversion stays in `src/unix.rs`.
- I/O and wait operations that may poll receive a monotonic caller-supplied `Deadline`.
- PTY input and output remain byte streams; terminal parsing accepts split controls and split UTF-8.
- Resizes use `TIOCSWINSZ`; tests do not call application resize or input internals.
- Protocol profiles are explicit allowlists; observed output never expands a profile.
- The semantic snapshot exposes only state the selected backend can represent.

## Topic docs

- System boundary and module responsibilities: [docs/architecture.md](docs/architecture.md).
- Public types, environment, input, and terminal-state API: [docs/api.md](docs/api.md).
- Spawn, I/O, signals, and process cleanup: [docs/process-lifecycle.md](docs/process-lifecycle.md).
- Snapshots, failure bundles, and replay: [docs/failure-artifacts.md](docs/failure-artifacts.md).
- Backend limits: [docs/backend-capabilities.md](docs/backend-capabilities.md).
- Output controls and query replies: [docs/protocol-profiles.md](docs/protocol-profiles.md); reviewed consumer traces: [docs/protocol-inventory.md](docs/protocol-inventory.md).
- Differential terminal traces: [tests/fixtures/differential/README.md](tests/fixtures/differential/README.md).
- Build, test, CI, and release workflow: [docs/development.md](docs/development.md).
- Historical dependency, platform, and consumer evidence: [docs/baselines.md](docs/baselines.md).
