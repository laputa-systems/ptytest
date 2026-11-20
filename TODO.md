Prefix: PTY
Next: 4
### PTY-3 · P3 · Measure representative PTY workloads

- **Area:** performance · benchmarks
- **Where:** PTY spawn, output waits, terminal processing and failure replay
- **Problem:** No benchmark harness measures end-to-end latency, throughput, memory or attribution for real PTY journeys, so functional tests cannot detect performance regressions.
- **Recommendation:** Build representative completed-work workloads with correctness oracles, resource attribution and interleaved baseline comparisons; provide tiered preflight and selectors once a harness exists.
- **Fix with:** bench-pyramid
- **Effort:** M
