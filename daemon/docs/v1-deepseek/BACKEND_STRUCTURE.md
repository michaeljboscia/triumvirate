# BACKEND_STRUCTURE — DeepSeek 4th-sibling integration (v1)

## High-level
A new module **`mcp-bridge::deepseek`** holds the HTTP runner + SSE parser + ghost-success
guard; a sibling **`mcp-bridge::deepseek_resilience`** holds the semaphore + breaker +
classification (modelled on `agy_resilience.rs` but NOT a generic copy). `triumvirate::agent_exec`
gains a `"deepseek"` arm in `run_named_agent_with_session_and_model` that calls the runner and
returns `ParsedAgentResult`. `shared-types::AskAgentRequest` gains four optional `deepseek_*`
overrides. `token-economics` gains a pub `ensure_deepseek_prices(db)` helper in
`attribution.rs` (production seed for two price rows in the existing `price_table` SQL
schema at `storage.rs:45` — NO schema change) and exposes `calculate_cost_usd` as `pub`.

## Workspace touch list (concrete file plan)

| Crate | File | Change |
|-------|------|--------|
| `mcp-bridge` | `src/lib.rs` | extend `is_supported_agent_name` to accept `"deepseek"`; export the new `deepseek` + `deepseek_resilience` modules; add env helpers — ALL 15 knobs (BASE_URL, API_KEY, MODEL, MAX_TOKENS, THINKING, REASONING_EFFORT, READ_TIMEOUT_SECS, TIMEOUT_SECS, TCP_KEEPALIVE_SECS, MAX_CONCURRENT, MAX_RPM, REASONING_CAP_TOKENS, LOG_DIR, LOG_REASONING_CAP_BYTES, BULK_BYTES). |
| `mcp-bridge` | `src/deepseek.rs` *(NEW)* | HTTP runner: request builder, reqwest client config (read_timeout/timeout/tcp_keepalive), POST /v1/chat/completions, SSE byte stream consumer, ghost-success guard, finish_reason guard, typed failure type carrying optional usage, return `ParsedAgentResult` on success or typed `Err` with usage on failure. |
| `mcp-bridge` | `src/deepseek_resilience.rs` *(NEW)* | `Semaphore` (default cap 8), token bucket (default 60 RPM), `BreakerState` enum with `HardOpenInsufficientBalance` / `OpenTransient { until: Instant, attempts: u32 }` / `HalfOpen { lease: Instant }` variants; classification fn `classify(status: u16) -> Classification { Hard, Transient, Local }`. |
| `triumvirate` | `src/agent_exec.rs` | new `"deepseek"` arm in `run_named_agent_with_session_and_model` (~1216) → calls `mcp_bridge::deepseek::run(...)`. **NEW:** `attempt_schedule` for `agent=="deepseek"` returns `1` (bypasses the generic 3-attempt loop at ~373). **NEW:** persist-before-`Err` path gated to DeepSeek typed failure with populated usage (REQ-DS-026 blast-radius safeguard). **NEW:** prewarm slot for `deepseek` is a safe no-op (~1291 area). |
| `triumvirate` | `src/main.rs` | `/status` `supported_agents` list extended with `"deepseek"` (~1874); session-spawn error text (~2008) gains `"deepseek"`. |
| `shared-types` | `src/lib.rs` | `AskAgentRequest` gains 4 optional fields: `deepseek_thinking: Option<DeepSeekThinking>`, `deepseek_reasoning_effort: Option<DeepSeekEffort>`, `deepseek_include_reasoning: Option<bool>`, `deepseek_max_tokens: Option<u32>`. New enums `DeepSeekThinking { Enabled, Disabled }` and `DeepSeekEffort { High, Max }` (low/medium→high server-side, but we accept all and normalize). |
| `mcp-tools` | `src/inter_agent.rs` | display name "deepseek" → "DeepSeek"; supported-agents fallback list (~275) gains `"deepseek"`. Tool schema for `ask_agent` exposes the new optional fields; tool description gains the anti-bulk soft note. |
| `token-economics` | `src/attribution.rs` (new pub `ensure_deepseek_prices` helper + production seed call at startup; also make `calculate_cost_usd` pub), `src/lib.rs` (re-exports) | Seed `deepseek-v4-pro` (input_per_mtok=0.435, cached_per_mtok=0.003625, output_per_mtok=0.87) + `deepseek-v4-flash` (0.14/0.0028/0.28) via idempotent INSERT OR IGNORE. The `price_table` SQL schema at storage.rs:45 is unchanged. The existing INSERT pattern at attribution.rs:156 is `#[cfg(test)]`-only — production needs the new helper. |
| `agent-worker` (if needed) | `src/lib.rs` | `require_reused_worker` treats DeepSeek's missing remote session_id as success (synthetic accounting id only). |
| `triumvirate` | `tests/deepseek_contract.rs` *(NEW)* | `#[ignore]` env-gated integration probe battery (REQ-DS-017). |
| `daemon/docs/` | `deepseek-operator-runbook.md` *(NEW)* | runbook (REQ-DS-018). |

## Traits / interfaces
This integration introduces **no new public traits** in v1. The runner's return type is the
existing `agent_adapter::ParsedAgentResult`; the failure type is `mcp_bridge::deepseek::Error`,
which is a concrete enum that the dispatch layer matches on.

```rust
// mcp-bridge/src/deepseek.rs (sketch)
pub async fn run(
    cfg: &DeepSeekConfig,        // env-loaded
    req: &DeepSeekRequest,       // built from AskAgentRequest + overrides
    events_tx: &EventsSender,    // for lifecycle/progress events
    resilience: &DeepSeekResilience,
) -> Result<ParsedAgentResult, DeepSeekFailure>;

pub struct DeepSeekFailure {
    pub kind: DeepSeekFailureKind,
    pub usage: Option<TokenUsage>,   // populated when stream produced anything billable
    pub message: String,
}

pub enum DeepSeekFailureKind {
    HardProvider(u16),               // 400/401/402/422 (402 → HardOpenInsufficientBalance)
    TransientProvider(u16),          // 429/5xx/503 (after internal retry budget exhausted)
    IdleTimeout,                     // 60s no bytes
    AbsoluteCeiling,                 // 1800s outer tokio timeout
    RunawayReasoning(u64),           // optional cap fired (carries observed reasoning tokens)
    BadFinishReason(FinishReason),   // length/content_filter/null
    GhostSuccessEmbedded(String),    // {"error":...} inside 200 stream
    NetworkPreFirstByte,             // already retried once internally; failed both
    NetworkMidStream,                // dirty disconnect after at least one byte
}

pub enum FinishReason { Stop, Length, ContentFilter, Other(String) }
```

## Module layout (after build)
```
daemon/crates/
├── mcp-bridge/src/
│   ├── lib.rs                       (gate + env helpers extended)
│   ├── deepseek.rs                  (NEW — runner + SSE parser + ghost-success + finish_reason)
│   └── deepseek_resilience.rs       (NEW — semaphore + breaker + classify)
├── triumvirate/src/
│   ├── agent_exec.rs                (deepseek arm; attempt_schedule=1; persist-before-Err gated)
│   ├── main.rs                      (/status + session-spawn extended)
│   └── tests/deepseek_contract.rs   (Rust integration test path: `<crate>/tests/`, NOT `src/tests/` — Wave-0 probe battery, #[ignore])
├── shared-types/src/lib.rs          (AskAgentRequest 4 optional fields + 2 enums)
├── mcp-tools/src/inter_agent.rs     (display name + supported-agents + tool schema)
├── token-economics/src/attribution.rs   (NEW pub `ensure_deepseek_prices(db)` + price seed; pub `calculate_cost_usd`)
└── agent-worker/src/lib.rs          (require_reused_worker accepts synthetic deepseek session_id)
```

## Data model: token persistence on success AND DeepSeek failure
- **Today:** `persist_daemon_token_record` writes a token record only on `Ok(parsed)` (agent_exec.rs ~187/~581); `cost_usd: None` (cost computed later by attribution).
- **v1 change (DeepSeek-gated):** for the DeepSeek arm only, if the runner returns
  `Err(DeepSeekFailure)` with `usage: Some(...)`, `execute_ask_agent` persists a token record
  *before* returning `Err`. The record carries `usage_source = "exact"` when the API's final
  usage chunk arrived, else `"estimated"` (bytes-received/4 + prompt estimate).
- **Blast-radius guard:** Gemini/Codex/Claude error paths are UNCHANGED — they don't carry a
  typed failure with usage, so they take no new persistence path. A regression-guard unit test
  asserts a non-deepseek `Err` writes zero token records.

## Streaming SSE handling (the critical parser)
Hand-rolled parser over `reqwest::Response::bytes_stream()`:

```
loop {
  bytes = await stream.next() within read_timeout(60s)   // resets on every chunk
  buffer.append(bytes)
  for line in buffer.drain_lines() {
    if line.starts_with(":") → THROTTLED keep-alive event (≤1/30s) + continue
    if line == "data: [DONE]" → close: parse_finish() → finalize/return
    if line.starts_with("data: ") {
      json = parse(line[6..])
      if "error" in json → classify embedded error → fail loud  // REQ-DS-029
      if "choices" empty AND "usage" present → capture usage → continue
      for choice in json.choices {
        delta = choice.delta
        if delta.reasoning_content → append to reasoning_acc + emit first_reasoning event
        if delta.content → append to content_acc + emit first_answer_token event
        if choice.finish_reason → capture
      }
      if reasoning_cap_tokens > 0 AND reasoning_acc.estimated_tokens > cap → abort local // REQ-DS-028
    }
  }
}
finalize:
  if finish_reason ∉ {stop} → BadFinishReason loud-fail (with usage if present)  // REQ-DS-030
  return Ok(ParsedAgentResult { response: content_acc, … })
```

Wrapped by `tokio::time::timeout(absolute_ceiling, …)` for REQ-DS-007 (b).

## What CoT bifurcation does to ParsedAgentResult
- The default response payload returned via `ask_agent` carries final `content` ONLY.
- The captured `reasoning_content` is written to a per-request JSON log file at
  `$TRIUMVIRATE_DEEPSEEK_LOG_DIR/<request_id>.json` (default `$HOME/.triumvirate/deepseek-logs/`),
  containing `{request_id, model, system_fingerprint, reasoning_content (size-capped
  256KB default), content, usage, cost_usd, finish_reason, timestamp}`. **NOT**
  `OutboxEvent.detail` (that would bloat the outbox). Owned by IMPLEMENTATION_PLAN T-009.
- When `deepseek_include_reasoning=true` is passed in `AskAgentRequest`, the response includes
  the reasoning trace inline (Claude's choice; defaults off).

## Stateless contract impl
- Synthetic `session_id = format!("deepseek-{uuid_v4}")` returned in `ParsedAgentResult`.
- Inbound `session_id` ignored.
- prewarm slot for "deepseek" returns `Ok(())` without spawning.
- `require_reused_worker` treats DeepSeek's missing remote session as success.

## Anti-bulk byte intercept
At the daemon entry point for `ask_agent` (before reaching the runner), if `agent == "deepseek"`
and `message.len() > _BULK_THRESHOLD_BYTES` (default 16384), return a clear error to Claude
("DeepSeek is remote+metered — payload too large; use a local sibling for bulk data"). Surface
the threshold env knob `TRIUMVIRATE_DEEPSEEK_BULK_BYTES`.

## Failure-class → breaker behavior (single source for builders)
(Reproduced verbatim from the canonical spec §4 — builders should reference this table when
implementing T-004 / T-007 / T-009 / T-010 / T-013.)

| Class | Trips provider breaker? | Meters? |
|-------|------|------|
| 402 (incl. ghost-success `Insufficient Balance`) | YES — `HardOpenInsufficientBalance` | yes |
| 429 / 5xx / 503 (incl. embedded) | YES — `OpenTransient` (threshold+cooldown) | yes if usage rcvd |
| Idle/read-timeout (60s, no bytes) | YES — `OpenTransient` (dead-stream) | estimated |
| Absolute ceiling (1800s) | NO — local orchestrator abort, fail loud | estimated |
| Runaway reasoning cap (if enabled) | NO — local abort, fail loud | estimated |
| Bad `finish_reason` (length/content_filter/null) | NO — loud fail (budget/policy/anomaly) | exact if usage rcvd else estimated |

## Cargo dependencies (no NEW runtime crates required; some test deps may be added)
- `reqwest` (already workspace dep) — used directly.
- `tokio` (workspace dep) — `Semaphore`, `time::timeout`.
- `serde` / `serde_json` (workspace deps) — request/response/SSE chunk JSON.
- `uuid` (already used elsewhere) — synthetic session_id.
- `bytes` / `futures-util` (workspace) — stream consumption.
