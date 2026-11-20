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

**Tests inventory at `606027b` (macOS arm64)**

| Public contract and critical decisions | Witnesses and remaining gaps |
| --- | --- |
| Direct exec, PTY ownership, size and resize, signals, EOF, hangup, child and group cleanup | `tests/pty_boundary.rs`: controlling terminal, spawn failure, resize/SIGWINCH, SIGINT, EOF/hangup, descendant and zombie leader, descriptor and SIGPIPE tests; `tests/io_regressions.rs`: cleanup and drop after errors. Escaped process groups and every OS failure remain untested. |
| Deadlines, output, quiet periods, bounded reads and writes | `src/config.rs` deadline equality; `tests/io_regressions.rs` idle/new bytes, quiescence, expired writes, flood and budget. Kernel interruption and boundary races are not exhaustively tested. |
| Bytes, terminal cells, modes, split UTF-8 and controls, explicit profile and replies | `src/terminal.rs` partition and differential corpus, `src/protocol.rs` profile tests, `tests/pty_boundary.rs` split CSI and query reply tests. The differential corpus covers shared backend behavior, not every VT sequence. |
| Environment, configuration and snapshots | `src/config.rs` size, locale, environment and scratch tests; `tests/pty_boundary.rs` attribute comparison. All combinations and filesystem failures are untested. |
| Failure bundles, JSONL and replay; replay CLI | `tests/pty_boundary.rs` timeout bundle and drop preservation; `src/replay.rs` ranges, schema, escapes and query/reply validation; `tests/replay_cli.rs` help. CLI error exits and malformed bundle combinations are not exhaustively tested. |

The supported CI shapes are Linux arm64 and macOS, normal and single-threaded
suite; this inventory is not evidence for other Unix targets. Critical
regions are expiry comparisons, quiescence, profile admission and reply
ordering, limits, child and process-group reaping, and replay validation.
The retained selector covers `src/terminal.rs`; it is unchanged. Hosted
controls and trial results are in `docs/deslop-terminal-replies.md`.

Dispositions by behavior, separating historical trial counts from this
inventory: deadline trial at `d60bf6d` caught 13, with five unviable and
no survivors or timeouts. Terminal trial at `b5ad4b9` caught 13, with five
unviable and no survivors or timeouts on each platform. Output-wait trial
at `6a31b11` had two equivalent true guards and one untested poll-timeout
false guard; the subsequent simplification removed these guards and caught
all four remaining identities. Control-only survivors (four constant cursor
positions, seven of 11 uncovered branch identities, and additional
configuration identities under whole-file fallback) demonstrate selector
sensitivity, not candidate results. Configuration survivors were not individually
classified: accepted risk for the advisory fallback, not a clean mutation run.
Lifecycle, replay, remaining configuration and broader protocol mutations
were not run; their behavior is untested by mutation, not declared equivalent.
No timeout is recorded in the completed deadline or terminal trials; wider
unrun regions have no timeout result. Unsupported environments: non-Linux,
non-macOS Unix and local Linux execution in this step need environment proof.

Acceptance is `make test`, `make lint`, hosted platform and single-threaded
suite, with `make coverage` and the retained `make mutants-diff` lane as
existing evidence. Historical coverage at `dc8840b` is 3000/3775 lines and
294/506 branches, not a current measurement. Local cargo-llvm-cov and
cargo-mutants are unavailable (Needs environment); no new mutation or coverage
claim is made. Ground's fresh 12.26s is a loaded-host reference, not a paired
performance gate; no current paired suite-cost claim is made (Needs environment).
No CI or retained scope change was made.

Consumers: output-wait candidate `e900a50` passed lz `722d74c`, e `b29c901`
and ish `fc62468` with path overrides; xsh `5f41e9a` failed to compile on
both base and candidate on macOS (`docs/deslop-output-wait.md`). Earlier #13
and #15 exercised fz and tm, and #13 exercised e, lz; exact scope and base
failures are recorded above. Soulseek-rs uses 0.3 and was not tested against
this candidate; xsh remains Needs environment or upstream repair. Remaining
consumer release qualification is untested, not a passing result. This final
inventory changes no runtime behavior and therefore requires no fresh consumer
run. No important new gap was demonstrated that warrants a new test.

## Tickets

### DSLP-5 · P2 · Dead code: tools and debloat

- **Area:** deslop · dead code
- **Where:** whole repository
- **Problem:** No dead-code scan has run.
- **Recommendation:** Run the deslop Dead code step. Accept: tools clean or allowlisted with reasons; walk-backs filed as Needs owner tickets.
- **Fix with:** deslop
- **Effort:** M

