# Baseline and validation record

This file records the evidence collected before the four consumer migrations.
The commands were run in fresh detached worktrees at the listed commits, so
their results do not rely on the uncommitted migration edits in the active
consumer worktrees.

## Dependency authorization

The implementation received explicit user approval for the production
dependencies `libc` and `vt100 = 0.16.2`. The user subsequently approved
`alacritty_terminal = 0.26.0` as a test-only differential oracle. It is not a
runtime dependency of the public PTY harness.

## Local platform

The local implementation and acceptance suite were run on macOS `Darwin
25.5.0`, `aarch64-apple-darwin`, with `rustc 1.99.0-nightly
(89c61a754 2026-07-23)`. The same pinned toolchain ran the complete shared
suite in an isolated `aarch64-unknown-linux-gnu` container on Linux
`7.0.14-orbstack-00380-ga7e0a2dc9535`, under normal and single-threaded Cargo
scheduling. The pinned CI workflow repeats the same suite, including the
UTF-8 locale check, on current macOS and Ubuntu runners.

## Pristine consumer baselines

| Consumer | Detached commit | Focused command | Result |
| --- | --- | --- | --- |
| `tm2` | `2d31550a1212fdf7cdd6d435c015a8489c8460f4` | `cargo test --test attached_client_pty` | 1 passed |
| `pi-agent-tui` | `ff56b64fd8bd92b58b7a3a955e7229621b617a30` | `cargo test -p pi-agent-tui --features pty-harness --test pty_streaming` | 1 passed; Cargo reported the pre-existing `nix 0.28.0` future-incompatibility warning |
| `e` | `1b5ea3010277ccbb2ce4c165fd2e4eb8a18090a8` | `cargo test --test e2e` | 142 passed, 1 ignored |
| `ish` | `924b88384c5ac6bfb38f85b859fe3b2cbedd8e28` | `cargo test --test pty` | 115 passed, 1 ignored |

The migration validation repeats each command after its consumer is adapted,
then runs the consumer's normal full suite. Keep unrelated failures separate
from PTY-harness evidence.

## Post-migration verification

On the local macOS platform, the shared crate passed `cargo test --all-targets`
under normal and `--test-threads=1` scheduling (23 unit tests and 14 PTY
acceptance tests). Each migrated focused suite also passed under both
schedules. The normal full suites then passed for all consumers:

| Consumer | Full command | Result |
| --- | --- | --- |
| `tm2` | `cargo test` | passed |
| `pi-agent-core-rs` | `cargo test` | passed |
| `e` | `cargo test` | passed (817 unit, 7 integration, 140 E2E passed; 1 E2E ignored) |
| `ish` | `cargo test` | passed (92 unit, 127 integration, 113 PTY passed; 1 PTY ignored) |

The `e` and `ish` focused PTY counts are two lower than their pristine
baselines because each removed two duplicate private `vt100` parser tests with
the old harness. Their exact split-clear/home, wide-Unicode, SGR, and erasure
contracts now live once in
`src/terminal.rs::shared_backend_retains_split_clear_home_and_unicode_erase_semantics`,
alongside the exhaustive terminal-model and profile chunk-boundary tests.

The Darwin zombie-leader descendant regression passed 50 consecutive isolated
runs after the process-group fallback fix; the complete shared suite passed 20
consecutive normal-scheduling runs and one final serialized run.
