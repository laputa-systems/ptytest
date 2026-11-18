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

## Tickets



### DSLP-3 · P2 · Types: strict profile and typing findings

- **Area:** deslop · types
- **Where:** type checker and lint configuration; Rust sources
- **Problem:** The strict-profile candidates in the deslop skill's tools reference have not been measured here.
- **Recommendation:** Run the deslop Types step. Accept: clean at the adopted profile, escape hatches not increased, witnesses recorded.
- **Fix with:** deslop
- **Effort:** M

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

