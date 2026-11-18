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

## Tickets




### DSLP-4 · P2 · Tests: coverage, boundary gaps, mutation

- **Area:** deslop · tests
- **Where:** test suites and CI
- **Problem:** Coverage and mutation results are unmeasured; no deslop mutation lane exists.
- **Recommendation:** Run the deslop Tests step. Accept: no undispositioned survivor in critical code, the test cost budget holds, and the lane is green where the tools are mirrored.
- **Fix with:** deslop
- **Effort:** M

### DSLP-5 · P2 · Dead code: tools and debloat

- **Area:** deslop · dead code
- **Where:** whole repository
- **Problem:** No dead-code scan has run.
- **Recommendation:** Run the deslop Dead code step. Accept: tools clean or allowlisted with reasons; walk-backs filed as Needs owner tickets.
- **Fix with:** deslop
- **Effort:** M

