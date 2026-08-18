# Protocol profile policy

`xterm-minimal-v1` is the initial versioned output profile. Its scope is
deliberately limited to the semantic needs proved by this crate's deterministic
fixture:

| Family | Semantic reason | Positive fixture evidence | Boundary |
| --- | --- | --- | --- |
| C0 CR/LF/BS/TAB/BEL | text layout and ordinary line output | startup/input acknowledgements | other C0 controls fail |
| ESC save/restore/index/reset/keypad | ordinary terminal lifecycle | parser unit coverage | other ESC finals fail |
| CSI cursor/editing/erasure/SGR | visible semantic screen | snapshot/parser coverage | non-parameter bytes fail |
| DEC private 1, 25, 47, 1047-1049, 2004 | cursor, alternate-screen, bracketed-paste lifecycle | cursor/alternate fixture paths | unlisted private modes fail |
| DEC private 1000/1002/1003/1004/1006 | tm's explicit mouse/focus shutdown contract | `tm attached client`, offsets 2326/2350 | unlisted private modes fail |
| DEC private 2026 | e's synchronized-output redraw bracketing | `e editor E2E`, offset 54 | unlisted private modes fail |
| CSI `>0u` | tm's explicit kitty-keyboard disable on detach | `tm attached client`, offset 2358 | other `u` prefixes fail |
| DA, DSR, CPR queries | deterministic terminal-peer behavior | split CPR fixture path | absent profile fails immediately |
| OSC 0/2 title and OSC 7 `file://` | harmless window metadata and ish working-directory publication | `ish interactive shell`, OSC 7 at offset 8 | clipboard and all other OSC fail |

All unlisted OSC/DCS/private-mode extensions remain rejected. The crate fixture
is provenance for the initial parser/reply behavior only; it is not
authorization to enable the profile for a consumer application. The reviewed
consumer inventory is recorded in [protocol-inventory.md](protocol-inventory.md).
