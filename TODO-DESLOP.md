Prefix: DSLP
Next: 6

Deslop campaign backlog ([plan](https://github.com/laputa-systems/stuff/blob/main/docs/campaigns/deslop.md), tier Calibration). One agent works this file with the `deslop` skill: take the lowest-numbered open ticket whose area does not start with `Needs owner ·`. After DSLP-5, work the open P1 and P2 findings, then fill in `## Report` below.

## Report

Ground at `3fd48af` found a deadline equality failure that allowed a write to
proceed on the expiry tick. [#13](https://github.com/laputa-systems/ptytest/pull/13)
adds a deterministic regression and passes hosted PTY acceptance on Linux
arm64 and macOS. The candidate was loaded through a Cargo path override in
the e, fz, tm, and lz consumer suites. The Ish denv case failed with both the
candidate and the released dependency. The xshi suite hit a pre-existing macOS
build error. Soulseek-rs uses the 0.3 series.

The shared Make entry points passed all 66 tests and Clippy on macOS arm64
after integration with `bed1e5d`. The Verification section maps the library
and replay CLI contracts to their tests.

Diagnose at `ece3a61` reviewed the child startup, deadline, terminal reply,
failure bundle, and replay boundaries. [#15](https://github.com/laputa-systems/ptytest/pull/15)
fixed the reproduced artifact overwrite. A real PTY timeout now retains
its original operation and error through cleanup. The macOS arm64 suite
passed 67 tests and Clippy. Linux and macOS acceptance passed in #15.

The #15 path override passed fz's default-TTY case, four tm attached cases,
and lz's named less 668 parity case. The focused e and ish failures also
occurred with base `5176d94`. The linked PR records their commands and
consumer revisions. Soulseek-rs and xsh were outside that run. Types,
broader mutation inventory and consumer release qualification remain open.

Types at `0edcf81` used Clippy on all targets and features with the pinned
nightly `5b7b37aef` on macOS arm64. Add `-W unsafe_code` and `-W
clippy::<rule>` for each candidate to the ordinary Clippy command, replacing
`--deny warnings`. Deduplicate JSON diagnostics by rule, file and line.

| Candidate | Locations | Decision |
| --- | ---: | --- |
| `unsafe_code` | 122 | Unix PTY process and descriptor functions need FFI. |
| `wildcard_enum_match_arm` | 4 | Process and terminal paths deliberately group fallback variants. |
| `match_wildcard_for_single_variants` | 0 | No demonstrated witness. |
| `cast_possible_truncation` | 5 | Includes bounded termios and polling values and explicit native integer widths. |
| `cast_sign_loss` | 7 | Includes return values checked for errors before conversion. |
| `unwrap_used`, `expect_used` | 276, 40 | Test assertions dominate. No adopted production witness. |
| `redundant_clone` | 1 | A diagnostic allocation falls outside the type contract. |
| `needless_pass_by_value` | 4 | Includes owned error and replay representations. |

The ordinary profile remains clean. No new escape hatch or runtime change.
Linux acceptance uses the existing hosted suite. The alternate Linux
backend was outside this macOS candidate measurement.

The first Tests coverage slice at `dc8840b` runs all 67 cases on macOS
arm64 with `make coverage`, cargo-llvm-cov 0.9.1 and nightly
`5b7b37aef`. It measures 3,000 of 3,775 lines and 294 of 506 branches
across `src/`, including inline tests. The integration source and
dependency paths are outside the report. The instrumented child gets
only `LLVM_PROFILE_FILE` from the test runner's coverage environment,
so profiles stay in the coverage target. No profile appeared in the
repository root on the final run. Linux-only decisions and mutation
qualification remain open.

## Tickets




### DSLP-4 · P2 · Tests: coverage, boundary gaps, mutation

- **Area:** deslop · tests
- **Where:** Deadline and child lifecycle in src/config.rs, src/io.rs and src/unix.rs, terminal protocol processing, replay, tests and hosted CI
- **Problem:** The coverage target, output-wait scenario and Linux/macOS acceptance are recorded in the Report and docs/deslop-output-wait.md. All four identities in the simplified output wait were caught. The affected lz, e and ish consumers passed. Xsh reaches the same macOS compile error with the base and candidate.
- **Recommendation:** Bound mutation to deadline relations and quiescence, terminal reply admission and failure reporting, read and trace limits, child and process-group reaping, and replay bundle behavior. Use io_regressions.rs, pty_boundary.rs and replay_cli.rs as the public witnesses. Qualify the remaining Linux and macOS union, targeted consumers, retained workflow controls and paired suite cost.
- **Fix with:** deslop
- **Effort:** M

### DSLP-5 · P2 · Dead code: tools and debloat

- **Area:** deslop · dead code
- **Where:** whole repository
- **Problem:** No dead-code scan has run.
- **Recommendation:** Run the deslop Dead code step. Accept: tools clean or allowlisted with reasons; walk-backs filed as Needs owner tickets.
- **Fix with:** deslop
- **Effort:** M

