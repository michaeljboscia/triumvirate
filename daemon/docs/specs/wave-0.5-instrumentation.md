# Wave 0.5 — Daemon Trace Instrumentation Spec

**Status:** Draft (pre-goatrodeo), 2026-04-19
**Authors:** Mike Boscia + Claude Opus 4.7
**Context:** ADR-001 r3 Wave 0.5 — instrument the live Triumvirate daemon's emission points to emit `TraceEvent`s (per ADR-001 Appendix B) into the JSONL sink shipped by the `trace-capture` crate. Output becomes the corpus the trace-replay stress profile consumes to produce honest Tier 1 SLO numbers.

---

## Goal

When `TRIUMVIRATE_TRACE=1` is set in the daemon's environment, every tool call, worker state change, peer-review event, and cost telemetry point emits a `TraceEvent` to `~/.triumvirate/traces/<YYYY-MM-DD>.jsonl`. When the env var is unset (default), zero emissions, zero overhead.

## In scope (this wave)

| Crate | Events |
|---|---|
| `agent-adapter` | `tool_call.started`, `tool_call.completed`, `cost.token_usage`, `cost.api_call` |
| `fleet` | `worker.spawned`, `worker.state_changed`, `worker.completed` |
| `peer-review` | `peer_review.requested`, `peer_review.decided` |

## Explicitly out of scope (defer to later waves)

- MCP bridge path instrumentation (separate crate, higher surface)
- HTTP server self-telemetry
- Dispatch-layer internals beyond spawn/state-changed/completed
- `lesson.candidate` events (separate worker dispatch — requires lesson-capture hooks that don't exist yet)

---

## Architecture

**Dependency injection.** Each in-scope crate receives `trace_sink: Arc<dyn TraceSink>` through its existing constructor/builder. The `TraceSink` trait lives in `trace-capture` and has one implementation today: `JsonlSink`. Disabled mode is a `NoopSink` struct returned from the resolver when `TRIUMVIRATE_TRACE` is unset.

**Emission pattern.** Fire-and-forget via bounded tokio `mpsc` channel (capacity 4096). Callsite does `trace_sink.emit(TraceEvent::new(...))` — cost is one `channel::try_send` ≈ sub-µs on the happy path. A dedicated writer task owned by the sink drains the channel and writes to the JSONL file. Callsite never blocks on disk I/O.

**Failure policy.**
- Channel full → drop event, increment `trace_dropped_full` counter, log at WARN once per 1000 drops.
- I/O error on write → log at WARN, increment `trace_dropped_io` counter, keep draining channel.
- Disk free < 5% at startup → sink initializes as `NoopSink` + prints one-line WARN to stderr.
- Disk free < 5% detected during run (checked every 60s) → stop draining until free > 10%, drop incoming.
- Never panic. Never silently swallow.

**Startup ordering.** Daemon `main.rs` constructs the sink FIRST, before any subsystem that might emit. Subsystem constructors accept the sink as a required argument; no lazy init.

**Shutdown.** On SIGTERM/SIGINT, drain the channel with a 5s budget, flush the file, close. If channel isn't empty at 5s, log `trace_dropped_on_shutdown` count.

---

## Event payload schemas (per-type typed structs)

Published in `trace-capture::payloads` module. Wave 0.5 ships with explicit Rust structs per event_type — not `serde_json::Value`. Future event_types require adding a struct (conformance gate). Example:

```rust
pub struct ToolCallCompleted {
    pub agent: String,           // "claude" | "codex" | "gemini"
    pub tool_name: String,
    pub session_id: Uuid,
    pub duration_ms: u64,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub success: bool,
    pub error_kind: Option<String>,
}
```

Each struct is `Serialize`; `TraceEvent::new` takes `impl Serialize` payload and converts.

## Correlation ID conventions

| Domain | correlation_id semantics |
|---|---|
| `tool_call.*` + `cost.*` | `session_id` of the agent session |
| `worker.*` | `worker_id` (UUID assigned at spawn) |
| `peer_review.*` | `review_id` (UUID assigned at request) |

Cross-domain correlation (e.g. "which tool calls belong to which peer-review") is **not** solved in this wave. Future wave can add `parent_correlation_id`.

## Configuration

- `TRIUMVIRATE_TRACE` env var, read once at daemon startup.
  - unset or `0` → sink = `NoopSink`, zero overhead
  - `1` → sink = `JsonlSink` at default path
  - `debug` → same as `1` but lowers sampling filters (currently a no-op; reserved)
- `TRIUMVIRATE_TRACE_DIR` env var overrides sink output dir (default `~/.triumvirate/traces`).
- No config-file integration in this wave (Tier 0 consolidation is separate).

## Hot-path overhead budget

- NoopSink emit: **< 100 ns** (ideally compiled out via `if cfg!(trace) || sink.enabled()`).
- JsonlSink emit (channel send success): **< 1 µs**.
- JsonlSink emit (channel full, drop): **< 500 ns** (non-blocking `try_send`).
- Writer-task serialization + write: off the hot path; target throughput 10k events/sec sustained.

## Per-worker scope (3 parallel workers)

| Worker | Crate | Files touched | Contract |
|---|---|---|---|
| H | `agent-adapter` | `src/lib.rs` + adapter modules | Add trace_sink parameter; emit tool_call.started before dispatch + tool_call.completed after response; emit cost.token_usage on token count hook + cost.api_call on HTTP response |
| I | `fleet` | `src/orchestrator.rs` + status transitions | Emit worker.spawned at launch, worker.state_changed on transitions (SPAWNED → WORKING → DONE/STUCK/FAILED), worker.completed on reap |
| J | `peer-review` | `src/lib.rs` | Emit peer_review.requested at insert, peer_review.decided at decision-commit |

**Shared Wave 0 prerequisite (not a worker):** Add `payloads` module + `TraceSink` trait + `NoopSink` impl to `trace-capture`. Fires before H/I/J. I'll write this personally (~30 min) so workers have stable contracts.

## Per-worker verification

Each worker adds a `#[tokio::test]` that:
1. Constructs its subsystem with a `MemorySink` (new test impl in `trace-capture::test_support`).
2. Exercises the emission point.
3. Asserts the expected `TraceEvent` landed in the in-memory buffer with correct `event_type`, `subject`, `correlation_id`, and payload shape.

Integration smoke at end of wave: start daemon with `TRIUMVIRATE_TRACE=1`, make one API call through each path (one tool call, one worker dispatch, one peer review), verify each produced the expected JSONL entries via `grep event_type`.

## Rollback plan

Set `TRIUMVIRATE_TRACE=0` (or unset). No code revert required. If the sink itself becomes a problem (e.g. writer task deadlock), revert the `trace-capture` workspace commit (`90c2f6e`'s successor).

---

## Open questions for /goatrodeo

1. Is the bounded mpsc + drop-on-full strategy sufficient, or do we need a ring-buffer fallback that never drops at the cost of overwriting oldest?
2. Should `NoopSink` emissions be compile-time gated (feature flag) or runtime-branched (cheap Arc<dyn> dispatch)?
3. Correlation ID cross-domain linkage — is the "not this wave" deferral acceptable, or will Tier 1 SLO analysis genuinely need it on day one?
4. Payload typed-structs vs `serde_json::Value` — r3 Codex critique said value is too loose. Is the typed approach above rigorous enough, or do we need a schema registry with versioning rules?
5. Disk-full 5%/10% thresholds — arbitrary or grounded?
6. Shutdown drain budget of 5s — right number?
7. 10k events/sec throughput target — realistic given JSONL serialization overhead at that rate?
