# PTY output wait mutation evidence

`PtyTest::wait_for_output` reports whether it read new bytes before the
supplied deadline. The real PTY fixture now proves three states. An
already-expired idle wait returns false. A future idle wait preserves
existing output and uses its full deadline. A new fixture acknowledgment
adds bytes and wakes the reader. The test observes the full readiness line
before measuring idle behavior.

At `6a31b119`, cargo-mutants 27.1.0 with the pinned Rust nightly on
macOS arm64 selected eight identities. The source blob was
`1b0288e531e93dc08267193efa756acd7df8972b` in both runs. With the
test blob `608a01cceeeb6edda56d2ab9bc8faf0c9af85059`, the same
selection changed from two caught and six missed in 66 seconds to five
caught and three missed in 62 seconds. Both initial suites passed.
The new test catches the missing read-timeout conversion and both
inverted output-length comparisons.

| Remaining identity in `src/io.rs` | Disposition |
| --- | --- |
| 295:54, guard replaced with true | Equivalent. The read path emits Timeout only when the passed deadline expires, including a terminal reply written under that same deadline. |
| 308:54, guard replaced with true | Equivalent. The poll path emits Timeout when it can no longer obtain remaining time from that same deadline. |
| 308:54, guard replaced with false | A timeout during poll would escape as an error. The idle fixture normally returns from the kernel poll and detects expiry before entering that guard. This branch still needs a controlled interruption or clock-boundary witness. |

Reproduce on clean source archives of the base and candidate. Set
`TMPDIR` to a scratch directory alongside both archives so the mutation
tool's temporary copy sits outside the archive it copies. Install the
pinned cargo-mutants 27.1.0 tool in a disposable prefix.

```sh
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo mutants \
  --file src/io.rs --re 'wait_for_output' --jobs 1 \
  --build-timeout 120 --timeout 15 --cargo-arg=--locked \
  --output OUTPUT
```

Use an outer ten-minute timeout and a separate output directory per run.
Compare `mutants.out/outcomes.json` by identity and require both unmutated
suites to pass. This selection covers only the output wait method.
