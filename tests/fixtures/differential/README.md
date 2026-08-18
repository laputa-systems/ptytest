# Canonical differential trace corpus

Each file records exact output chunks (`bytes:`, hexadecimal) and resize
operations (`resize: columns rows`). `src/terminal.rs` replays the same chunks
through `vt100 0.16.2` and `alacritty_terminal 0.26.0`; a second test compares
the primary backend's final snapshots to `canonical-vt100.ptytest` on macOS
and Linux.

| Trace | Provenance | Semantic purpose |
| --- | --- | --- |
| `startup-and-cursor` | first-party fixture | clear/home, cursor movement, and editing |
| `alternate-lifecycle` | first-party fixture | alternate screen and cursor visibility restoration |
| `unicode-and-erasure` | first-party fixture | split wide/combining UTF-8 and erasure |
| `resize-explicit-boundaries` | first-party fixture | resize with explicit line boundaries; soft-wrap reflow is intentionally outside the v1 primary capability documented in `docs/backend-capabilities.md` |
| `mode-lifecycle` | first-party fixture | bracketed paste, application cursor/keypad, and mouse lifecycle |
| `tm2-detach` | `tm2` `attached_client_pty`, commit `2d31550a1212fdf7cdd6d435c015a8489c8460f4`, output offsets 2326/2350/2358 | detach cleanup controls |
| `e-synchronized-redraw` | `e` `e2e` startup, commit `1b5ea3010277ccbb2ce4c165fd2e4eb8a18090a8`, output offset 54 | xterm synchronized-output bracket |
| `ish-working-directory` | `ish` interactive PTY startup, commit `924b88384c5ac6bfb38f85b859fe3b2cbedd8e28`, output offset 8 | OSC 7 working-directory metadata |

The consumer traces are deliberately retained as observed fragments, not
expanded into a permission to admit new controls. See
`docs/protocol-inventory.md` for the review decision behind each one.
