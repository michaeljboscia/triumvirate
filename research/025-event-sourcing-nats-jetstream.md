# Research 025: Event Sourcing with NATS JetStream

**Every agent decision, every debate turn, every tool call — immutable append-only log with replay.**

## What JetStream Gives Us for Event Sourcing
- Append-only streams = immutable event log
- At-least-once delivery + exactly-once with deduplication
- Optimistic concurrency control (expected last sequence)
- Consumer replay policies: from beginning, from timestamp, only new
- Pull-based and push-based consumers
- Built-in API audit events ($JS.EVENT.ADVISORY)

## How This Maps to Triumvirate
Streams:
- `debate.arguments` — every Toulmin claim/rebuttal
- `tasks.events` — task lifecycle (created → assigned → debated → approved → executed → completed)
- `tools.requests` + `tools.results` — every tool invocation and outcome
- `governance.decisions` — every OPA policy evaluation
- `agents.state` — agent health, context window usage, error events

Replay use cases:
- Daemon crashes → restart → replay from last checkpoint → resume exactly where we were
- "What happened in that debate?" → replay `debate.arguments` from task_id
- "Why was this tool call made?" → trace `governance.decisions` → `tools.requests` → `tools.results`
- Debugging: replay entire session event by event

## Key Design Decisions
- Monotonic sequence numbers per stream (not wall-clock)
- Per-agent consumer cursor for exact resume position
- Snapshot every N events for fast recovery (don't replay 10K events on restart)
- WAL-style recovery: snapshot + replay tail

## Sources
medium.com, james-carr.org, alamrafiul.com, oneuptime.com, byronruth.com, nats.io
