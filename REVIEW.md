# Review: ptytest

Scope: crate source, tests, documentation, CI, and release tooling. Checks run: `cargo check --locked --lib` passed; `mdtool pointers` reported 0 findings; `mdtool check --untracked AGENTS.md README.md REVIEW.md docs/*.md tests/fixtures/differential/README.md` completed with 0 errors and path/reference warnings; macOS probes reproduced the cleanup, process-enumeration, and quiescence findings. No repository tests or CI jobs were run. Not covered: Linux execution, consumer repositories, and dependency internals or advisories.

## Summary

The crate keeps real PTY I/O separate from the screen model and protocol validation. The reliability findings from this review are resolved in the current tree: process cleanup continues through terminal-processing failures, process enumeration handles complete lists, quiescence and I/O honor deadlines, replay validates event records, snapshot updates use unique temporary files, and the CLI and maintainer commands follow their expected behavior. Focused regressions cover those changes. R12 tracks deterministic coverage for the Darwin process-list retry path. Documentation routes end users and maintainers to the right level of detail, while build and release commands are documented.

## Findings

### R12 · P3 · Darwin process-list retries lack deterministic coverage

- **Area:** Tests
- **Where:** `src/unix.rs:221-263`
- **Problem:** The Darwin process-list test only checks that enumeration includes the current process. It does not force a full buffer or verify that the returned PID count controls truncation, so the retry and truncation branches that protect descendant cleanup can regress without a deterministic failure.
- **Recommendation:** Exercise the capacity and returned-count decisions with controlled values so tests cover a full-buffer retry and truncation without relying on live process-table growth.
- **Fix with:** isolate-effects
- **Effort:** S (hours)

## Strengths to keep

- Tests exercise a real PTY and child process instead of application input or resize internals.
- Raw traces, a semantic terminal model, and protocol validation remain separate evidence surfaces.
- Protocol profiles are explicit allowlists, and the default profile denies validation and replies.
- CI covers macOS and Linux with normal and single-threaded test scheduling.
