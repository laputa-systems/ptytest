# PTY read deadline evidence

The continuous-output fixture exercises the public wait and timeout
error on a real PTY. Its assertion checks that the error retains the
requested 20 ms budget and reports an elapsed interval at least as
long. The fixture already proves the reader saw output before the
wait and observed more after it.

At `d60bf6dc`, cargo-mutants 27.1.0 selected 18 identities in the
`Deadline` implementation. The existing tests caught 11, with two
survivors and five unviable builds. The new assertion caught both
remaining identities, which replace the configured duration with
`None` or zero. The final run caught 13 with five unviable builds,
no misses and no timeouts. The unchanged source blob is
`2d046e2746e17438300da504815f5d5d7e5b2b86`. All 68 ordinary tests
and Clippy passed on macOS arm64.

The unviable alternatives need a nonexistent default deadline or
multiply an instant by a duration. The exact `--list` inventory
hash is `7358e067221826b330a0d242b5319b9644360a4da9367802b5707907185fbb71`.
Use a disposable checkout and the pinned Rust toolchain.

```sh
TMPDIR="$PWD/.work/tmp" cargo mutants --no-config --file src/config.rs \
  --re 'Deadline::' --in-place --timeout 20 --build-timeout 180 \
  --output .work/tmp/deadline-witness --no-times -- --lib --test io_regressions
```

The other PTY I/O, replay and whole-change decisions remain in DSLP-4.
