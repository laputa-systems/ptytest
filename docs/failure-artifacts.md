# Snapshots and failure artifacts

## Semantic snapshots

`ScreenSnapshot` is the owned representation used by assertions and snapshot files. Default snapshots cover visible screen semantics and selected lifecycle modes. `SnapshotOptions::with_attributes()` adds coalesced non-default cell attributes. The backend's limits, including the absence of soft-wrap reflow, are described in [backend capabilities](backend-capabilities.md).

Snapshot assertions do not write during normal runs. Set `PTYTEST_UPDATE_SNAPSHOTS=1` explicitly to update a snapshot; each update creates an exclusive temporary sibling file and renames it into place. A failed replacement removes its temporary file. A mismatch includes an inline diff and writes expected, actual, and diff files to the failure bundle.

## Failure bundles

Failures and dropping a live harness write evidence under `target/ptytest-failures/<scenario>-<pid>-<counter>/`. Bundles include redacted command/configuration text, a versioned ordered `events.jsonl` trace with byte ranges, readable `events.log`, raw `input.bin` and `output.bin` bytes up to the configured trace limit, the semantic screen and terminal state, exit status, and a failure description. Snapshot mismatches also include expected, actual, and diff files.

`secret_env` values and secret-looking environment keys are redacted in text diagnostics. Raw input/output files are intentionally exact, so tests should use fake credentials. The trace capacity defaults to 8 MiB and can be changed on `Scenario`; exceeding it returns `TraceLimitExceeded` instead of silently truncating evidence.

Timeout and assertion errors identify the scenario, operation, elapsed time, process status, dimensions, and artifact path. Their bundles add the semantic screen, terminal state, configuration, and recent event history.

## Offline replay

`ptytest-replay <failure-bundle>` reconstructs the semantic screen from recorded reads and resizes. It never executes the recorded application. `--dump-events` prints the ordered events; `--step` emits checkpoints; `--event N` and `--byte O` stop at an event or output-byte offset; `--help` prints usage. Replay validates complete version 1 event records, rejects unknown fields and event kinds, checks event ordering and contiguous input/output ranges, and verifies recorded terminal-query/reply traffic.
