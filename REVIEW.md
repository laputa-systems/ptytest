# Review: ptytest

Scope: crate source, tests, documentation, CI, and release tooling. Checks run: `cargo check --locked --lib` passed; `mdtool pointers` reported 0 findings; `mdtool check --untracked AGENTS.md README.md REVIEW.md docs/*.md tests/fixtures/differential/README.md` completed with 0 errors and path/reference warnings; macOS probes reproduced the cleanup, process-enumeration, and quiescence findings. No repository tests or CI jobs were run. Not covered: Linux execution, consumer repositories, and dependency internals or advisories.

## Summary

The crate has a clear boundary between real PTY I/O, the independent screen model, and protocol validation. The main reliability risks are in process cleanup and wait semantics: reachable trace failures can leave a child alive, Darwin cleanup enumerates only a fraction of host processes, and quiescence can return true too early. The documentation now routes end users and maintainers to the right level of detail, while the build, test, and release commands are directly represented.

## Findings

### R7 · P3 · The lint target applies fixes to the worktree

- **Area:** Build
- **Where:** `Makefile:6-7`
- **Problem:** `make lint` runs `cargo clippy --fix --allow-dirty` across all targets and features. A routine lint command can rewrite tracked files, including in a dirty worktree.
- **Recommendation:** Make `lint` check-only and put autofixes behind a separately named explicit target if needed.
- **Fix with:** none
- **Effort:** S (hours)

### R8 · P3 · Release recovery backups default to `/tmp`

- **Area:** Operability
- **Where:** `tools/bump_minor_release.sh:207-211`
- **Problem:** When `TMPDIR` is unset, release backups go to `/tmp`. That conflicts with the repository working-file rule and can lose recovery copies on hosts where `/tmp` is RAM-backed or cleared on reboot.
- **Recommendation:** Store the backups under ignored `.work/tmp`, or require a suitable caller-provided `TMPDIR`.
- **Fix with:** os-boundaries
- **Effort:** S (hours)

### R9 · P3 · Attribute snapshot coverage does not pin serialized styles

- **Area:** Tests
- **Where:** `tests/pty_boundary.rs:613-626`, `src/terminal.rs:195-210`
- **Problem:** The integration golden is generated from the same unstyled screen it compares, while the unit test checks only that an attribute range exists. Incorrect serialized style values can pass both tests.
- **Recommendation:** Serialize a styled screen and assert the literal attribute range and style values.
- **Fix with:** test-audit
- **Effort:** S (hours)

### R10 · P3 · Timeout test has a host-scheduling upper bound

- **Area:** Tests
- **Where:** `tests/pty_boundary.rs:388-406`
- **Problem:** The test fails if a 1 ms timeout reports more than 25 ms. A scheduler pause can exceed that threshold even when timeout behavior is correct, making CI depend on host load.
- **Recommendation:** Verify elapsed-time semantics with a controllable clock, or remove the tight upper bound while retaining the deadline regression check.
- **Fix with:** test-audit
- **Effort:** M (days)

### R11 · P3 · Replay CLI treats `--help` as a bundle path

- **Area:** Operability
- **Where:** `src/bin/ptytest-replay.rs:5-8`, `src/bin/ptytest-replay.rs:42-70`
- **Problem:** The first argument is always parsed as a failure-bundle path. Running `ptytest-replay --help` tries to read `--help/configuration.txt` and returns a path error instead of usage.
- **Recommendation:** Handle `--help` before parsing the bundle path and print the usage text with a successful exit status.
- **Fix with:** none
- **Effort:** S (hours)

## Strengths to keep

- Tests exercise a real PTY and child process instead of application input or resize internals.
- Raw traces, a semantic terminal model, and protocol validation remain separate evidence surfaces.
- Protocol profiles are explicit allowlists, and the default profile denies validation and replies.
- CI covers macOS and Linux with normal and single-threaded test scheduling.
