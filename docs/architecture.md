# Architecture

`ptytest` runs a compiled interactive program at its terminal boundary. The harness owns a Unix kernel PTY, starts the requested executable directly, sends input bytes and kernel resizes, and receives the program's terminal output.

Each output chunk advances three independent views: the exact raw trace, a protocol scanner and query responder, and the `vt100` semantic terminal model. The semantic model is the visual assertion surface; raw bytes and protocol events explain wire-level behavior. Protocol validation does not infer permission from what the model accepts.

## Module boundaries

- `src/config.rs` owns scenario, command, environment, dimensions, and deadlines.
- `src/unix.rs` contains PTY, process, signal, and platform-specific system calls.
- `src/io.rs` owns the harness, nonblocking I/O, and ordered event trace.
- `src/terminal.rs` keeps the terminal-model implementation behind a private boundary; `src/snapshot.rs` owns its public semantic view.
- `src/protocol.rs` owns profile validation and deterministic terminal-query replies.
- `src/artifact.rs` writes failure evidence; `src/replay.rs` reconstructs it offline.

The runtime dependencies are `libc` and `vt100 = 0.18`. `alacritty_terminal` is used only by the differential test oracle. Windows/ConPTY, remote connections, shell parsing, GUI or pixel output, fonts, mouse abstraction, recording/GIF output, asynchronous APIs, and soft-wrap reflow are outside the current contract.
