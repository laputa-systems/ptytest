# Terminal cursor replies

The PTY fixture writes three characters, queries the cursor and writes
two more characters in the same output. The child observes row 2,
column 4 in the reply, and the input artifact contains the exact reply
bytes. The two bytes after the query would move the cursor to column 6
if processed before the reply.

At `b86cf09`, the previous fixture expected row 2, column 2. Cargo-mutants
27.1.0 replaced the zero-based cursor result with the constant `(1, 1)`,
and the test passed. The changed fixture caught that identity, raising
detection from 12 to 13 of the same 18 selected mutations. The five
others were unviable. Both runs used terminal source blob
`89a46db2705972c65f9bae40bea9702c17127610` and produced the same inventory
SHA-256 `92a131d4ac7b35501b8a325ed4f343a5c6ba0c5e3e0e137aac879df5b5984c9b`.

In a disposable checkout, set `TMPDIR` to `.work/tmp` and run
`cargo mutants --no-config --file src/terminal.rs --timeout 25 -- --locked`.
This scope exercises the terminal adapter and its PTY cursor reply.

## Hosted terminal mutation trial

The [positive trial](https://github.com/laputa-systems/ptytest/actions/runs/44276455149)
at `b5ad4b9` ran cargo-mutants 27.1.0 on Linux ARM64 and macOS. Both
accounted for all 18 identities, with 13 caught and five unviable. Gate
times were 50.2 and 113.5 seconds. The two hosted jobs used 238 runner
seconds. Both ordinary PTY jobs and lint passed.

| Control | Result on both platforms |
| --- | --- |
| [Documentation edit](https://github.com/laputa-systems/ptytest/pull/26) | Both jobs explicitly selected no mutations. |
| [Cursor test removal](https://github.com/laputa-systems/ptytest/pull/27) | Four constant cursor positions survived out of 18. |
| [Removal with an independent accessor edit](https://github.com/laputa-systems/ptytest/pull/28) | At `5e82ee5`, the four cursor positions survived and both changed `Size::columns` identities were caught in the same job. |
| [Uncovered branch](https://github.com/laputa-systems/ptytest/pull/29) | Seven of 11 changed-region identities survived. |

Ordinary tests and lint passed every control. The first mixed edit
touched a constructor the native mutator omits. Whole-file fallback
selected 117 identities, including further configuration survivors.
The repeated mixed control used the independently selected accessor.
Local driver checks also rejected a missing outcome file, an empty
relevant selection and a zero-second budget.

Use an ignored `.work/`, Python 3.14 and cargo-mutants 27.1.0, then
run `make mutants-diff BASE=origin/main` from a clean checkout.
The first hosted checkout missed the local scratch exclusion and could
not copy the workspace. The trial now verifies the exclusion before
creating scratch. Lifecycle, replay and the wider configuration
inventory remain in DSLP-4.

The workflow watches terminal implementation, PTY tests and fixtures,
the gate, build settings, and documentation. A watched test or
settings change adds the full terminal scope. Related production edits
join that scope even when they are in other files. Unrelated production
edits alone use the ordinary CI jobs until their mutation scopes are
qualified.

The terminal mutation workflow uses the same ordinary PTY tests as the terminal reply witness.
