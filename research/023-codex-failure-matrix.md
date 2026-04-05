# Research 023: Codex's Failure Matrix — What Will Break

## 10 Things That Will Break First
1. CLI protocol drift (stream-json schema changes across versions)
2. Backpressure collapse (slow consumer stalls everyone)
3. TTY/PTY weirdness (buffering, color codes, prompts differ)
4. Process zombie/orphan leaks (restart leaves detached children)
5. Half-open streams (process alive but no events flowing)
6. Interleaving race bugs (concurrent stdin writes = malformed JSON)
7. State divergence after restart (agents resume with mismatched turns)
8. Idempotency failures (retry causes duplicate side effects)
9. Clock skew/ordering (wall-clock creates wrong causality)
10. Terminal rendering contention (flicker, starvation, unreadable)

## 10 Edge Cases
- Agent emits invalid JSON mid-stream then recovers
- CLI outputs mixed text + JSON in same stream
- Agent hangs on shutdown, blocks session cleanup
- SIGINT during write transaction to event log
- "Ghost output" arrives after task cancellation
- Context window rollover during long session
- Human edits while workflow in-flight
- Subscription expiry while subprocess alive
- Rate-limit throttling looks like normal latency
- Terminal resize/reattach (tmux reconnect) during collaboration

## State + Recovery (must design now)
- Event-sourced session log with monotonic sequence numbers
- Per-agent cursor (last_applied_seq) for exact resume
- Heartbeats + progress watchdogs
- Supervisor tree for subprocess lifecycle
- Idempotency keys on every side-effecting action
- Saga compensation for partial workflows
- Crash-safe snapshots every N events + WAL tail replay
- Session rehydration protocol

## 12 More Searches Codex Wants
1. Claude CLI stream-json framing, schema versioning, cancellation
2. Gemini Live API reconnection + session resume
3. Anthropic Go SDK SSE retry and event-id resume
4. Go subprocess pipes + context cancellation + process groups
5. NATS JetStream exactly-once with dedupe windows
6. Temporal idempotency with external subprocess activities
7. BubbleTea/tcell concurrent streaming panes
8. PTY vs pipe behavior for AI CLIs
9. WAL + snapshot patterns (Badger/SQLite/Postgres)
10. OpenTelemetry GenAI conventions for streaming tokens
11. Chaos engineering for stream truncation and process death
12. Credential persistence for subscription CLIs in daemons
