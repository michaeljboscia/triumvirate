# Research 028: Codex Break Points — Batch 2 (Health, Races, Idempotency, Recovery)

## Break Point 5: Half-Open Streams — SOLVED
**Three-tier health model for subprocesses:**
- **Liveness:** PID exists + `cmd.Wait()` goroutine hasn't returned. Simple, lightweight, don't check deps.
- **Readiness:** Liveness passes + subprocess producing output within timeout + accepting stdin writes.
- **Progress:** Readiness passes + output contains expected event types within SLA window.

**Implementation:**
- I/O stream monitoring: timeout on stdout reads. No data in N seconds = "alive but stuck"
- Heartbeat injection: periodically write lightweight query to stdin, expect response
- Go libraries: `heptiolabs/healthcheck` (separates liveness/readiness, exposes HTTP endpoints)
- Key: timeouts on ALL I/O, concurrent dependency checks, graceful shutdown on failure

## Break Point 6: Interleaving Race Bugs — SOLVED
**Two approaches, channel is better for our use case:**

1. **sync.Mutex:** Wrap stdin pipe in SafeWriter. Lock before write, unlock after. Simple but couples producers to writer.
2. **Channel (dedicated writer goroutine):** Single goroutine reads from `chan []byte`, writes to stdin exclusively. Producers marshal JSON, send to channel. Natural backpressure via buffered channel.

**Winner: Channel pattern.** Matches our NATS-centric architecture — everything flows through channels/topics already. Each agent's connector has a dedicated writer goroutine for its CLI stdin. NATS messages fan into the channel, writer serializes to pipe.

**Critical:** Always append `\n` after each JSON object for NDJSON framing.

## Break Point 7: NATS JetStream Exactly-Once — SOLVED
**Server-side deduplication + idempotent consumers:**

Publisher side:
- Set `Nats-Msg-Id` header with stable idempotency key per message
- Configure `--dupe-window` on stream (default 2min, configurable)
- Server rejects duplicate msg IDs within the window

Consumer side:
- Durable consumers with `AckExplicit` policy
- `AckSync()` for double-acknowledgment (server confirms your ack)
- `MaxAckPending` for backpressure
- Idempotent sink logic: UPSERT/ON CONFLICT for DB ops, KV store check for other side effects
- `AckWait` tuned to 2-3x P95 processing latency
- `AckProgress()` to extend timeout for long-running tasks

## Break Point 8: Temporal Crash Recovery — SOLVED (This is Temporal's ENTIRE PURPOSE)
**Event sourcing + deterministic replay:**

1. Temporal stores immutable Event History for every workflow
2. On daemon crash/restart: new worker receives Event History
3. Worker replays workflow code from beginning against event history
4. Completed activities are NOT re-executed — results loaded from history
5. Workflow resumes from exact point of failure

**Activity heartbeats for long-running agent calls:**
- `activity.RecordHeartbeat(ctx, progressDetails)` — periodic signal to Temporal
- `HeartbeatTimeout` configured per activity — if missed, activity marked failed
- On retry: heartbeat details available for resuming from last checkpoint
- This handles our "agent is thinking for 30 seconds" case perfectly

**Determinism requirement:** Workflow code must use `workflow.Now()`, `workflow.Go()`, `workflow.Channel` instead of native Go constructs. The Go SDK handles this.

## Sources
stackoverflow.com, github.com, dalibo.com, oneuptime.com, medium.com, redhat.com, nats.io, synadia.com, temporal.io, corneliadavis.com, go.dev
