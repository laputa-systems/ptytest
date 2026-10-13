# Development and release

## Checks

Run the pinned toolchain from `rust-toolchain.toml`:

```sh
cargo build --locked --all-targets
cargo test --locked
cargo clippy --locked --all-targets --all-features -- --deny warnings
```

CI runs Clippy and the test suite on Ubuntu and macOS, with normal and single-threaded test scheduling. A scheduled job also runs the terminal differential corpus on both platforms. The test-only `alacritty_terminal` oracle is not part of the runtime dependency set.

`make test` runs `cargo test --locked`. `make lint` runs Clippy in check-only mode and does not rewrite files.

## Test authoring

Synchronize on a screen predicate or fixture protocol, not a delay. Tests for elapsed-time behavior should use a controllable application clock or assert a domain event. Keep ordinary non-interactive CLI tests on pipes and application unit tests in their own suites; use this crate for PTY integration and terminal-protocol behavior.

Include a deterministic schedule seed in the scenario label so a failure bundle preserves the input needed to reproduce it.

## Release

`make publish` runs `tools/bump_minor_release.sh`. It checks Cargo registry ownership and the published package version, requires a clean working tree for a new release, updates the minor version, commits and tags it, and publishes through Cargo. It queries the default Cargo registry and requires network access. The script also handles retrying a release commit whose publish did not complete. Recovery copies go under `TMPDIR` when it is set, or `.work/tmp` otherwise.

## Evidence

The platform/toolchain and consumer-migration record is historical evidence in [baselines](baselines.md). Current platform coverage is defined by [the CI workflow](../.github/workflows/ci.yml).
