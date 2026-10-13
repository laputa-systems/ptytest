# ptytest

`ptytest` is a Unix-only Rust test-support crate for exercising interactive programs through a real PTY.

Build: `cargo build --locked --all-targets` · Test: `cargo test --locked` · Check: `cargo clippy --locked --all-targets --all-features -- --deny warnings`.

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
