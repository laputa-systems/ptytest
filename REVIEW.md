# Review: ptytest

Scope: crate source, tests, documentation, CI, and release tooling. Checks run: `cargo check --locked --lib` passed; `mdtool pointers` reported 0 findings; `mdtool check --untracked AGENTS.md README.md REVIEW.md docs/*.md tests/fixtures/differential/README.md` completed with 0 errors and path/reference warnings; macOS probes reproduced the cleanup, process-enumeration, and quiescence findings. No repository tests or CI jobs were run. Not covered: Linux execution, consumer repositories, and dependency internals or advisories.

## Summary

The crate keeps real PTY I/O separate from the screen model and protocol validation. The reliability issues identified by this review are resolved in the current tree: process cleanup continues through terminal-processing failures, process enumeration handles complete lists, quiescence and I/O honor deadlines, replay validates event records, snapshot updates use unique temporary files, and the CLI and maintainer commands follow their expected behavior. Focused regressions cover the behavior changes. Documentation routes end users and maintainers to the right level of detail, while build and release commands are documented.

## Strengths to keep

- Tests exercise a real PTY and child process instead of application input or resize internals.
- Raw traces, a semantic terminal model, and protocol validation remain separate evidence surfaces.
- Protocol profiles are explicit allowlists, and the default profile denies validation and replies.
- CI covers macOS and Linux with normal and single-threaded test scheduling.
