# Research 027: Codex Break Points — Batch 1 (Protocol Drift, Backpressure, PTY, Zombies)

## Break Point 1: CLI Protocol Drift — MITIGATED
**Strategy:** Tolerant consumer pattern. Parse with jsoniter, ignore unknown fields, embed version in schema.
- Use JSON Schema to define expected structure per CLI version
- Version-detect at daemon startup (check `claude --version`, `codex --version`)
- Automated drift detection: compare inferred schema against expected on each stream
- "Fail closed, not silent" — if breaking change detected, halt and report, don't silently corrupt

## Break Point 2: Backpressure Collapse — SOLVED
**Strategy:** Buffered channel between pipe reader and consumers.
- Dedicated goroutine reads from `io.PipeReader` → pushes to buffered `chan []byte`
- Buffer size configurable (default 100 chunks) — subprocess never blocks
- Each consumer reads from channel independently
- If channel fills, reader goroutine blocks — but subprocess stays unblocked because it only writes to the pipe
- MUST copy byte slices before sending to channel (prevent shared buffer races)
- Use `context.WithCancel` for graceful shutdown of the bridge goroutine

## Break Point 3: PTY vs Pipe — CRITICAL DESIGN DECISION
**When running as subprocess with pipes:**
- Programs detect non-TTY via `isatty()` → disable colors, switch to block buffering
- Block buffering means output NOT line-by-line — may cause delays
- Interactive features (raw mode, char-by-char input) won't work via pipes
- Some CLIs refuse to function without TTY

**Solutions:**
- Option A: Use `--output-format stream-json` flag to force JSON output (Claude supports this)
- Option B: Use PTY wrapper (Go's `creack/pty` package) to make subprocess think it's in a terminal
- Option C: Hybrid — PTY for interactive mode, pipes for structured JSON mode
- **For Claude:** `--print` mode with stream-json should bypass TTY detection. NEED TO VERIFY.
- **For Gemini/Codex:** NEED TO TEST if they output JSON when piped vs interactive

## Break Point 4: Process Zombies/Orphans — SOLVED
**Three-layer defense:**

1. **Setpgid:** `SysProcAttr{Setpgid: true}` — child in own process group. Kill with `syscall.Kill(-PID, SIGTERM)` to cascade to all descendants.

2. **Pdeathsig:** `SysProcAttr{Pdeathsig: syscall.SIGTERM}` — kernel sends SIGTERM to child if parent thread dies. CAVEAT: tied to thread, not process. Use `runtime.LockOSThread()` on spawning goroutine.

3. **Graceful shutdown handler:** Catch SIGINT/SIGTERM → send SIGTERM to all process groups → wait 5s → SIGKILL survivors → cmd.Wait() to reap zombies.

**For Docker:** If Go daemon runs as PID 1, use `go-reaper` library or `--init` flag to handle zombie reaping.

## Sources
endgrate.com, dev.to, api-university.com, stackoverflow.com, technori.com, hackernoon.com, medium.com, geeksforgeeks.org, sobyte.net, mezhenskyi.dev, go.dev, sigmoid.at, pocketbase.io
