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
