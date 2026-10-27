# Process lifecycle and I/O

## Spawn

The parent prepares the `execve` arguments and environment, opens the PTY at the requested size, and creates a close-on-exec error pipe before `fork`. The child closes the master, creates a session and controlling terminal, assigns and verifies foreground-process-group ownership, connects the slave to standard input/output/error, closes every descriptor above standard error except that error pipe, changes directory, restores `SIGPIPE` to the default disposition, and calls `execve`. Setup failures send a fixed error record and exit; the parent distinguishes them from an application exit. The session-leading child PID is retained as the original process-group identity.

Only `src/unix.rs` owns platform FFI details. Linux links `openpty` through `libutil`; Darwin uses `libproc` when group signaling needs a member-by-member fallback.

## Terminal I/O

The master is nonblocking. Reads and writes check the caller's monotonic deadline as they make progress. Partial writes handle retryable errors and poll for writability when needed; each interrupted poll recalculates the remaining time. Reads feed the exact bytes to the trace, semantic model, and protocol scanner. Readable data is drained before a hangup or error is treated as final, so trailing terminal-restoration bytes are retained; PTY `EIO` is treated as slave closure. The scanner and model handle controls split across reads; input and output fragmentation are separate behaviors.

Resizing issues `TIOCSWINSZ` only. The kernel delivers the resulting window-size change; the harness does not invoke application internals or synthesize a signal.

## Cleanup

`finish` and `Drop` attempt cleanup against the retained original process group. Cleanup observes the leader without reaping it, sends `SIGTERM`, waits for the leader and ordinary group members, escalates to `SIGKILL` when needed, then reaps the leader. A terminal-processing error during cleanup is preserved for the caller while cleanup continues through signal escalation and reaping. Keeping the leader unreaped prevents its numeric PID and process-group identity from being reused during cleanup; after reaping, the old numeric group ID is never signaled again. A descendant that starts a new session is outside this process-group cleanup boundary, and terminal restoration is not required after `SIGKILL`.

The Darwin fallback enumerates by PID count, grows a full buffer, and rechecks each PID's process group immediately before signaling it. Process inspection and signals remain platform-local so system-call and ABI details do not leak into the public API.
