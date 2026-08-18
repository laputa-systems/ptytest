# Reviewed consumer protocol inventory

These are reporting-mode observations reviewed before enabling
`xterm-minimal-v1` in the named migrated scenario. They are evidence for the
specific rule listed here, never automatic admission of future controls.

| Repository/scenario | Observed span | Semantic reason | Decision |
| --- | --- | --- | --- |
| `tm2` / `attached_client_pty` | output 2326 `CSI ?1000l`, 2350 `CSI ?1004l`, 2358 `CSI >0u` | an attached client releases mouse, focus, and kitty-keyboard state while detaching | admit the exact named selectors and `>0u`; reject other private selectors/prefixes |
| `pi-agent-tui` / `pty_streaming` | startup and clean exit lifecycle controls | alternate screen, cursor, and bracketed-paste lifecycle asserted by the test | existing alternate/cursor/paste rules pass unchanged |
| `e` / `e2e` startup | output 54 `CSI ?2026h` | redraws are bracketed by xterm synchronized-output mode | admit exactly DEC 2026; reject other unlisted private modes |
| `ish` / `pty` startup | output 8 `OSC 7;file://… BEL` | shell publishes its current working directory to the terminal | admit OSC 7 only with the `file://` form; retain OSC clipboard and other OSC rejection |

The corresponding representative tests enable `ProtocolProfile::xterm_minimal_v1()`.
