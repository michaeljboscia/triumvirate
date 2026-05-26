# IMPLEMENTATION_PLAN — DeepSeek 4th-sibling integration (v1)

> 18 tasks across 6 waves (Wave 0 → Wave 5). Strict wave ordering; tasks within a wave are
> parallel-safe unless an explicit `depends` chain says otherwise. Every task has all 8
> mandatory XML fields. The Execution Contract appendix at the bottom is part of this plan.

## Build overview
| Wave | Theme | Tasks |
|------|-------|-------|
| **0** | **Live API contract probe (Wave-0 gate)** | T-000 |
| 1 | Foundation (gate, env knobs, price rows) — parallel-safe | T-001, T-002, T-003 |
| 2 | Resilience + HTTP client base — parallel-safe | T-004, T-005 |
| 3 | Runner (SSE parser, guards, runaway, usage, top-level) | T-006 → T-010 (sequential by `depends`) |
| 4 | Dispatch + per-call surface + stateless + anti-bulk | T-011, T-012, T-013, T-014, T-015 |
| 5 | Verification re-run + runbook | T-016, T-017 |

---

## Wave 0 — Live API contract gate

```xml
<task id="T-000" req="REQ-DS-017" wave="0" depends="">
  <description>Wave-0 DeepSeek live contract probe battery. Ground-truths the real API
  (auth, models, streaming SSE shape, reasoning_content separation, usage cache-hit/miss,
  401, 422, finish_reason:length) BEFORE any production wiring is enabled.
  Prerequisite: a funded DeepSeek account (no auto-grant — $0 balance ⇒ 402).</description>
  <files>daemon/crates/triumvirate/tests/deepseek_contract.rs</files>
  <scope_out>Do NOT modify any production code in this task. Do NOT add the probes to the
    default `cargo test` run (they MUST be `#[ignore]` and env-gated). Do NOT bake the API
    key into the test source.</scope_out>
  <tools>cargo check, cargo test, file read/write within the new test file, env vars
    TRIUMVIRATE_DEEPSEEK_API_KEY + TRIUMVIRATE_DEEPSEEK_BASE_URL.</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>cargo test -p triumvirate -- --ignored deepseek_contract — hits
    api.deepseek.com with the operator's funded key; asserts the streamed response
    contains a reasoning_content chunk AND a content chunk AND a usage chunk with
    prompt_cache_miss_tokens populated AND a `[DONE]` sentinel. A stub server returning
    a single hardcoded JSON fails (no streaming, no [DONE], no usage chunk).</reality_test>
  <done_when>All probes in REQ-DS-017 pass against the funded account; the suite is
    `#[ignore]`-gated; results captured to PROBE_RESULTS.md for the audit trail.</done_when>
</task>
```

---

## Wave 1 — Foundation (parallel-safe)

```xml
<task id="T-001" req="REQ-DS-001,REQ-DS-013,REQ-DS-016,REQ-DS-022" wave="1" depends="">
  <description>Make `deepseek` a recognized agent everywhere the gate/surface is hardcoded:
  is_supported_agent_name, daemon /status supported_agents, session-spawn error text,
  execute_ask_agent error text, display_agent_name, inter_agent supported-agents fallback.
  deepseek is a new TOP-LEVEL name, NOT a GeminiBackend enum variant.</description>
  <files>daemon/crates/mcp-bridge/src/lib.rs, daemon/crates/triumvirate/src/main.rs,
    daemon/crates/triumvirate/src/agent_exec.rs, daemon/crates/mcp-tools/src/inter_agent.rs,
    daemon/crates/mcp-tools/src/lib.rs, daemon/crates/triumvirate/tests/integration_http.rs</files>
  <scope_out>Do NOT modify dispatch routing (T-012 territory). Do NOT add the deepseek HTTP
    runner. Do NOT touch token-economics. Do NOT change Gemini/Codex behavior in any way.</scope_out>
  <tools>cargo check, cargo test, file read/write within the files list.</tools>
  <verify>cargo check --workspace</verify>
  <reality_test>cargo test -p mcp-bridge supports_deepseek_name asserts
    is_supported_agent_name("deepseek")==true and ("claude")==false; an in-process /status
    test asserts the JSON response's supported_agents array contains "deepseek";
    AND two INDEPENDENT assertions (one per error path; the two are separate code paths and
    one alone leaves the other stale):
    (a) a SESSION-SPAWN request with an unknown agent returns an error whose supported-agents
    list explicitly contains "deepseek" (covers main.rs ~2008);
    (b) a POST /ask-agent HTTP request with an unknown agent returns an error response whose
    supported-agents list explicitly contains "deepseek" — covers agent_exec.rs ~247 via its
    PUBLIC HTTP surface (execute_ask_agent is pub(crate); /ask-agent is the correct integration-test surface).
    A stub that flips only the gate, or updates only ONE of the two error texts, fails this.</reality_test>
  <done_when>Every gate/surface hardcoding gemini/codex now also accepts deepseek; daemon
    /status lists "deepseek"; display name returns "DeepSeek".</done_when>
</task>

<task id="T-002" req="REQ-DS-002,REQ-DS-003,REQ-DS-015" wave="1" depends="">
  <description>Env helpers for DeepSeek config. Load ALL 15 env knobs (BASE_URL, API_KEY,
  MODEL, MAX_TOKENS, THINKING, REASONING_EFFORT, READ_TIMEOUT_SECS, TIMEOUT_SECS,
  TCP_KEEPALIVE_SECS, MAX_CONCURRENT, MAX_RPM, REASONING_CAP_TOKENS, LOG_DIR,
  LOG_REASONING_CAP_BYTES, BULK_BYTES) into a strongly-typed `DeepSeekConfig` struct with
  the documented defaults. T-002 is the single AUTHORITATIVE config-owner — all knobs loaded
  + tested here, even if consumed later by T-009 (LOG_*) or T-015 (BULK_BYTES). API_KEY MUST be
  a redacted-Debug wrapper (never logged, never in argv). Concrete embodiment of REQ-DS-002
  (BASE_URL = cloud api.deepseek.com/v1) + REQ-DS-003 (API-key guardrails) + the rest of REQ-DS-015.</description>
  <files>daemon/crates/mcp-bridge/src/deepseek_config.rs, daemon/crates/mcp-bridge/src/lib.rs</files>
  <scope_out>Do NOT make HTTP requests in this task. Do NOT shell out. Do NOT modify
    AskAgentRequest (that's T-011). Do NOT pass API_KEY via command-line flags.</scope_out>
  <tools>cargo check, cargo test, std::env, serde Debug derive customization.</tools>
  <verify>cargo check -p mcp-bridge</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_config_defaults — set TRIUMVIRATE_DEEPSEEK_API_KEY=secret-test,
    leave others unset, load DeepSeekConfig::from_env(); assert MAX_TOKENS==32768,
    TIMEOUT_SECS==1800, READ_TIMEOUT_SECS==60, THINKING==Enabled, REASONING_EFFORT==High,
    MAX_CONCURRENT==8, MAX_RPM==60, REASONING_CAP_TOKENS==0, BASE_URL=="https://api.deepseek.com/v1",
    LOG_DIR ends with "deepseek-logs/", LOG_REASONING_CAP_BYTES==262144, BULK_BYTES==16384;
    assert format!("{:?}", cfg.api_key) does NOT contain "secret-test".</reality_test>
  <done_when>DeepSeekConfig::from_env() returns a populated struct matching REQ-DS-015
    defaults; API_KEY's Debug is redacted.</done_when>
</task>

<task id="T-003" req="REQ-DS-009" wave="1" depends="">
  <description>Seed `price_table` rows for `deepseek-v4-pro` and `deepseek-v4-flash` IN
  PRODUCTION (NOT only test helpers — the existing INSERT pattern at attribution.rs:156 is
  inside `#[cfg(test)]` and is therefore test-only; a real seed mechanism is required).
  Concretely: add a PUB helper `pub fn ensure_deepseek_prices(db: &TokenDb) -> anyhow::Result<()>`
  to attribution.rs that performs idempotent `INSERT OR IGNORE` of both rows with the documented
  prices and an effective_date of today. Wire a call site at daemon startup (in `triumvirate`
  binary init) so production DBs are seeded on first run. Also make `calculate_cost_usd`
  PUBLIC (currently private at attribution.rs:47) — the runner needs it for synchronous
  per-consult cost computation (REQ-DS-021). No schema change (storage.rs:45 columns sufficient).</description>
  <files>daemon/crates/token-economics/src/attribution.rs, daemon/crates/token-economics/src/lib.rs, daemon/crates/triumvirate/src/main.rs</files>
  <scope_out>Do NOT modify the `token_records` schema. Do NOT modify the `price_table` schema
    in storage.rs. Do NOT alter the cost formula in calculate_cost_usd beyond adding `pub`.</scope_out>
  <tools>cargo check, cargo test, rusqlite for the INSERT statements.</tools>
  <verify>cargo test -p token-economics calculate_cost_usd</verify>
  <reality_test>cargo test -p token-economics deepseek_pricing — open a FRESH in-memory DB
    that does NOT have any price rows, call `ensure_deepseek_prices(&db)`, then query: assert
    both `deepseek-v4-pro` AND `deepseek-v4-flash` rows are present (the helper is the seed —
    a stub that no-ops fails this); call again → still 2 rows (idempotent); then
    `calculate_cost_usd` (pub) against a synthetic TokenRecord
    {model:"deepseek-v4-pro", input_tokens:1_000_000, output_tokens:1_000_000, cached_tokens:0}
    returns within ±$0.001 of $1.305; cached_tokens=1_000_000,input_tokens=0 within ±$0.001
    of $0.873625; flash variant within ±$0.001 of $0.42. A test-only INSERT pattern (like the
    existing #[cfg(test)] code) does NOT satisfy this test.</reality_test>
  <done_when>Pub `ensure_deepseek_prices(db)` exists; called at daemon startup so production
    DBs are seeded on first run; `calculate_cost_usd` is `pub`; numbers match spec pricing
    within FP tolerance; idempotency verified.</done_when>
</task>
```

---

## Wave 2 — Resilience + HTTP client base (parallel-safe within)

```xml
<task id="T-004" req="REQ-DS-006,REQ-DS-010" wave="2" depends="T-002">
  <description>New `deepseek_resilience.rs`: tokio Semaphore (default cap 8 from T-002 env),
  token bucket (default 60 RPM), breaker with three states (HardOpenInsufficientBalance,
  OpenTransient{until,attempts}, HalfOpen{lease}), and a classify(status:u16)→Classification
  function. Do NOT generic-copy agy_resilience.rs (its reset-window-cooldown doesn't map).</description>
  <files>daemon/crates/mcp-bridge/src/deepseek_resilience.rs, daemon/crates/mcp-bridge/src/lib.rs</files>
  <scope_out>Do NOT modify agy_resilience.rs. Do NOT make HTTP calls (T-010 wires them in).
    Do NOT couple the breaker to specific URL or method.</scope_out>
  <tools>cargo check, cargo test, tokio::sync::Semaphore.</tools>
  <verify>cargo test -p mcp-bridge deepseek_resilience</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_breaker — feed a sequence
    {Ok,Ok,Err(402)} to the breaker → assert state==HardOpenInsufficientBalance;
    feed {Err(429),Err(429),Err(429)} → assert state==OpenTransient with cooldown>0; advance
    a mock clock past cooldown → assert HalfOpen; classify(400)==Hard, classify(401)==Hard,
    classify(402)==Hard, classify(422)==Hard, classify(429)==Transient, classify(500)==Transient,
    classify(503)==Transient. A stub that returns Closed regardless fails the 402 case.</reality_test>
  <done_when>Breaker, semaphore, token bucket, and classify pass the synthetic-outcome
    suite; module is exported from mcp-bridge.</done_when>
</task>

<task id="T-005" req="REQ-DS-007" wave="2" depends="T-002">
  <description>HTTP client config + reqwest builder for DeepSeek. Construct
  reqwest::Client with read_timeout=60s, timeout=1800s, tcp_keepalive=30s from
  DeepSeekConfig.</description>
  <files>daemon/crates/mcp-bridge/src/deepseek.rs, daemon/crates/mcp-bridge/Cargo.toml</files>
  <scope_out>Do NOT implement the SSE parser or the runner. Do NOT make actual API calls in
    this task. Do NOT hardcode timeouts (they come from DeepSeekConfig). New dependencies
    (e.g. `hyper` for the in-process test server) MUST land as `[dev-dependencies]` in
    mcp-bridge/Cargo.toml; do NOT add them to other crates' Cargo.toml.</scope_out>
  <tools>cargo check, cargo test, reqwest::ClientBuilder, hyper test server (in-process).</tools>
  <verify>cargo check -p mcp-bridge</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_client_timeouts — spin up a local
    hyper-based test server that emits 1 byte every 5 seconds for 90 seconds; build a client
    with read_timeout=60s and call it; the request must COMPLETE (because every byte resets
    the rolling timer). A stub that ignores read_timeout and just uses `timeout` fails at
    the 60s mark.</reality_test>
  <done_when>`build_client(cfg)->reqwest::Client` exists and exhibits the rolling
    read_timeout behavior; tcp_keepalive set; absolute outer timeout applied at the runner
    level (T-010) via tokio::time::timeout.</done_when>
</task>
```

---

## Wave 3 — Runner (SSE parser, guards, runaway, usage, top-level)

```xml
<task id="T-006" req="REQ-DS-019" wave="3" depends="T-005">
  <description>SSE parser: consume reqwest bytes_stream; split on `\n\n` event boundaries;
  ignore `:` keep-alive lines; recognize `data: [DONE]`; for `data: <json>` parse JSON and
  accumulate delta.reasoning_content and delta.content separately; handle arbitrary
  chunk boundaries (incl. mid-JSON splits).</description>
  <files>daemon/crates/mcp-bridge/src/deepseek.rs</files>
  <scope_out>Do NOT implement ghost-success or finish_reason guards yet (T-007). Do NOT
    implement runaway abort (T-008). Do NOT make network calls.</scope_out>
  <tools>cargo check, cargo test, serde_json, bytes, futures-util.</tools>
  <verify>cargo test -p mcp-bridge deepseek_parser</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_parser_chunked — feed a synthetic byte
    sequence containing ": keep-alive\n\n", a reasoning chunk, a content chunk, a usage
    chunk, and `data: [DONE]\n\n` — split into pieces with one mid-JSON boundary. Assert:
    parser yields 1 keep-alive event, reasoning_acc and content_acc both populated
    correctly, usage extracted, DONE seen. A stub that buffers naively or assumes
    one-event-per-chunk fails.</reality_test>
  <done_when>Parser correctly handles cross-chunk JSON, keep-alive comment lines, the
    `[DONE]` sentinel, and emits accumulated content/reasoning + extracted usage.</done_when>
</task>

<task id="T-007" req="REQ-DS-029,REQ-DS-030" wave="3" depends="T-006">
  <description>Ghost-success detector + finish_reason loud-failure. For each parsed JSON
  chunk: if top-level `error` key present → classify embedded error (map error code/string
  to the same HTTP-status taxonomy) and return DeepSeekFailureKind::GhostSuccessEmbedded.
  At finalize: if finish_reason ∈ {length, content_filter, null, "unknown"} → return
  BadFinishReason — do NOT return Ok(parsed) with partial content.</description>
  <files>daemon/crates/mcp-bridge/src/deepseek.rs</files>
  <scope_out>Do NOT trip the provider breaker on bad finish_reason (budget/policy/anomaly,
    not provider-down). Do NOT pass partial gibberish to Claude.</scope_out>
  <tools>cargo check, cargo test, serde_json.</tools>
  <verify>cargo test -p mcp-bridge deepseek_guards</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_guards — (a) stream with
    {"error":{"code":"insufficient_balance","message":"Insufficient Balance"}} in the middle
    → finalize returns GhostSuccessEmbedded classified as Hard (like a 402);
    (b) stream ending finish_reason:"length" → finalize returns BadFinishReason(Length),
    NOT Ok(parsed); (c) finish_reason:"stop" → Ok. A stub returning Ok on (a) or (b) fails.</reality_test>
  <done_when>Ghost-success is detected per-chunk and routed through the same classification
    path as HTTP errors (no new breaker policy); bad finish_reasons block partial success.</done_when>
</task>

<task id="T-008" req="REQ-DS-028" wave="3" depends="T-006">
  <description>Optional runaway-reasoning early-abort. When cfg.reasoning_cap_tokens > 0:
  the parser loop maintains a running estimate of streamed reasoning tokens
  (reasoning_acc.chars()/4); when the estimate crosses the cap, the parser aborts the read,
  returns RunawayReasoning(observed_tokens). MUST NOT trip the provider breaker. Default
  cfg.reasoning_cap_tokens=0 (disabled).</description>
  <files>daemon/crates/mcp-bridge/src/deepseek.rs</files>
  <scope_out>MUST NOT trip the provider breaker. MUST NOT fire by default (cap=0). Do NOT
    consume the full stream after aborting — drop the future cleanly.</scope_out>
  <tools>cargo check, cargo test.</tools>
  <verify>cargo test -p mcp-bridge deepseek_runaway</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_runaway — with cfg.reasoning_cap_tokens=100
    and a stream emitting 1000 chars of reasoning_content, parser aborts with
    RunawayReasoning(~250) and the passed-in breaker remains Closed. With cap=0 (default),
    the same stream completes without aborting. A stub that ignores the cap fails (1);
    a stub that trips the breaker on this abort fails (2).</reality_test>
  <done_when>When cap>0 and observed reasoning chars/4 exceed cap, parser aborts cleanly
    with estimated metering populated and the breaker untouched.</done_when>
</task>

<task id="T-009" req="REQ-DS-009,REQ-DS-018,REQ-DS-021,REQ-DS-023,REQ-DS-026" wave="3" depends="T-003,T-006">
  <description>Three concerns through the same finalize path:
  (a) **Usage mapping** — output_tokens ← completion_tokens (already INCLUDES
  completion_tokens_details.reasoning_tokens — DO NOT double-add); input ← prompt_cache_miss_tokens;
  cached ← prompt_cache_hit_tokens. When the usage chunk is missing (disconnect/abort/bad
  finish_reason), record usage_source="estimated" with bytes_received/4 + prompt_estimate.
  (b) **Per-consult cost line** — synchronously compute cost via pub `attribution::calculate_cost_usd`
  (made pub by T-003) and emit a lifecycle cost line.
  (c) **DeepSeek per-request log record** — write a JSON file per request to
  `$TRIUMVIRATE_DEEPSEEK_LOG_DIR/<request_id>.json` (default `$HOME/.triumvirate/deepseek-logs/`,
  env from T-002) containing `{request_id, model, system_fingerprint, reasoning_content
  (size-capped 256KB by default — env `_LOG_REASONING_CAP_BYTES`), content, usage, cost_usd,
  finish_reason, timestamp}`. This is the NAMED storage target for REQ-DS-023 (CoT bifurcation)
  + REQ-DS-018 (`system_fingerprint` observability). NOT `OutboxEvent.detail`.</description>
  <files>daemon/crates/mcp-bridge/src/deepseek.rs, daemon/crates/mcp-bridge/Cargo.toml</files>
  <scope_out>Do NOT add columns to TokenRecord schema. Do NOT change attribution.rs beyond
    what T-003 already does. Do NOT implement Err-path token persistence here (T-013 owns
    that). Do NOT write the API key or the request `messages` payload into the per-request
    log file (privacy — only response artifacts).</scope_out>
  <tools>cargo check, cargo test, std::fs for the per-request JSON log, serde_json.</tools>
  <verify>cargo test -p mcp-bridge deepseek_usage_map</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_usage_map — given usage{prompt_tokens:18,
    completion_tokens:174, completion_tokens_details:{reasoning_tokens:120},
    prompt_cache_hit_tokens:0, prompt_cache_miss_tokens:18}: produces TokenUsage
    {input:18, cached:0, output:174} (NOT 174+120). With model="deepseek-v4-pro": cost ≈
    $0.00016 (within ±$0.0000001). A stub that adds reasoning_tokens to output overstates and
    fails. ALSO assert: a JSON log file exists at `$TRIUMVIRATE_DEEPSEEK_LOG_DIR/<request_id>.json`
    after the call, contains `reasoning_content`, `system_fingerprint`, `cost_usd` populated,
    and does NOT contain the API key or the request `messages` text (privacy regression guard).
    With no usage chunk and 800 bytes received → estimated record (bytes/4≈200) AND the log
    file still written with `usage_source: "estimated"`.</reality_test>
  <done_when>Exact mapping landed (no double-add); estimated fallback populated; lifecycle
    cost line emitted; per-request JSON log written to configured dir; reasoning capped to
    default 256KB; API key + messages NOT in the log (privacy verified by test).</done_when>
</task>

<task id="T-010" req="REQ-DS-004,REQ-DS-005,REQ-DS-008,REQ-DS-014,REQ-DS-024" wave="3" depends="T-004,T-005,T-006,T-007,T-008,T-009">
  <description>DeepSeek runner top-level — public `run(cfg, req, events, resilience) ->
  Result<ParsedAgentResult, DeepSeekFailure>`. Orchestrate: breaker check → semaphore
  acquire → build request body (model, thinking, reasoning_effort, max_tokens — REQ-DS-005)
  → POST /v1/chat/completions with stream=true → SSE parser → guards → finalize → return
  ParsedAgentResult on Ok OR typed DeepSeekFailure on Err. Wrap in
  tokio::time::timeout(absolute_ceiling) — the absolute SLA ceiling that fires loud
  (REQ-DS-024). All failure paths return Err with no sibling substitution (REQ-DS-008). One
  outer attempt — internal retries only for pre-first-byte network failure (1 retry) and
  429-with-Retry-After (bounded).</description>
  <files>daemon/crates/mcp-bridge/src/deepseek.rs, daemon/crates/mcp-bridge/Cargo.toml</files>
  <scope_out>Do NOT modify agent_exec wiring (T-012). Do NOT add a generic outer retry loop.
    Do NOT log the API key.</scope_out>
  <tools>cargo check, cargo test, tokio::time::timeout, wiremock-rs or hyper test server.</tools>
  <verify>cargo test -p mcp-bridge deepseek_runner</verify>
  <reality_test>cargo test -p mcp-bridge deepseek_runner — against a wiremock-rs/hyper mock
    server: (a) canned streaming reasoning→content→usage→[DONE] → run() returns
    Ok(ParsedAgentResult{ response: "<answer>", session_id: starts_with("deepseek-"),
    token_usage: {…} }); (b) 402 response → Err(DeepSeekFailure{kind:HardProvider(402),
    usage:None}); (c) 429 with Retry-After:1 followed by success → Ok (one internal retry
    honored); (d) mid-stream dirty disconnect → Err(NetworkMidStream) with estimated usage
    populated. A stub that ignores the mock or retries 3× fails.</reality_test>
  <done_when>run(cfg, req, events, resilience) integrates all of Wave 2/3; happy path +
    each major failure mode covered.</done_when>
</task>
```

---

## Wave 4 — Dispatch + per-call surface + persistence + stateless + anti-bulk

```xml
<task id="T-011" req="REQ-DS-027" wave="4" depends="">
  <description>AskAgentRequest gains 4 OPTIONAL fields: deepseek_thinking,
  deepseek_reasoning_effort, deepseek_include_reasoning, deepseek_max_tokens. New enums
  DeepSeekThinking{Enabled,Disabled} and DeepSeekEffort{High,Max,Xhigh} (we accept all
  variants and normalize; xhigh→max). MCP tool schema exposes them. Backward-compatible
  (Gemini/Codex ignore).</description>
  <files>daemon/crates/shared-types/src/lib.rs, daemon/crates/mcp-tools/src/inter_agent.rs</files>
  <scope_out>Do NOT consume these fields in the runner yet (that's T-012). Do NOT change
    the AskAgentRequest fields for gemini/codex. MUST stay backward-compatible.</scope_out>
  <tools>cargo check, cargo test, serde_json.</tools>
  <verify>cargo test -p shared-types ask_agent_request_optional_deepseek</verify>
  <reality_test>cargo test -p shared-types — deserialize {"agent":"gemini","message":"x"} →
    all 4 deepseek_* fields are None (backward-compat); deserialize
    {"agent":"deepseek","message":"x","deepseek_thinking":"disabled","deepseek_max_tokens":512}
    → populated; serde round-trip preserves values. A stub that drops the disabled value on
    round-trip fails.</reality_test>
  <done_when>4 optional fields present; MCP tool schema lists them; existing callers
    unaffected; round-trip preserves values.</done_when>
</task>

<task id="T-012" req="REQ-DS-014,REQ-DS-023" wave="4" depends="T-010,T-011">
  <description>agent_exec.rs deepseek arm in run_named_agent_with_session_and_model.
  Build DeepSeekRequest from AskAgentRequest (defaults + per-call overrides from T-011) →
  call mcp_bridge::deepseek::run(...) → map ParsedAgentResult. CoT bifurcation DEFAULT:
  .response carries final `content` only; reasoning_content is captured to a per-request
  log/ledger record. When deepseek_include_reasoning==Some(true): include reasoning in
  .response.</description>
  <files>daemon/crates/triumvirate/src/agent_exec.rs</files>
  <scope_out>Do NOT modify run_agent_process_with_session (subprocess-shaped, unchanged
    for v1). Do NOT change Gemini/Codex paths. Do NOT bypass the resilience check.</scope_out>
  <tools>cargo check, cargo test, wiremock-rs or hyper test server (reuse T-010's).</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>cargo test -p triumvirate deepseek_dispatch — against the mock:
    (a) ask_agent {agent:"deepseek", message:"x"} → AskAgentResponse with .response ==
    final content only (no reasoning text); a per-request log record contains the
    reasoning_content. (b) ask_agent {agent:"deepseek", message:"x",
    deepseek_include_reasoning:true} → .response includes reasoning. (c) ask_agent
    {agent:"gemini", message:"x"} → unchanged Gemini behavior. A stub that always includes
    reasoning fails (a); a stub that breaks gemini fails (c).</reality_test>
  <done_when>deepseek arm wired; ParsedAgentResult mapping landed; default response is
    content-only; include_reasoning opts in; Gemini/Codex untouched.</done_when>
</task>

<task id="T-013" req="REQ-DS-006,REQ-DS-010,REQ-DS-026" wave="4" depends="T-010,T-012">
  <description>execute_ask_agent — DeepSeek bypass-retry + persist-before-Err for DeepSeek
  typed failure. attempt_schedule for agent=="deepseek" returns 1 (bypasses the generic
  3-attempt loop ~373). When the deepseek arm returns Err(DeepSeekFailure{usage: Some(u)}),
  persist the token record BEFORE returning Err. **Blast-radius safeguard: this path is
  gated to DeepSeek typed failure only — Gemini/Codex/Claude Err paths are UNCHANGED.**</description>
  <files>daemon/crates/triumvirate/src/agent_exec.rs</files>
  <scope_out>MUST NOT change Gemini/Codex error paths or their token persistence. MUST NOT
    add retries for DeepSeek (1 outer attempt). MUST NOT persist Codex/Gemini Err records.</scope_out>
  <tools>cargo check, cargo test, mock runner that returns the typed failure type.</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>cargo test -p triumvirate execute_ask_agent_deepseek_errpath —
    (1) agent="deepseek" + runner returning Err(HardProvider(402), usage:Some(...)) → exactly
    ONE outer attempt (no retry), exactly ONE token record persisted with usage_source
    ∈ {exact,estimated};
    (2) agent="codex" + a failing runner → ZERO token records persisted (regression guard);
    (3) agent="deepseek" + runner returning Ok(parsed) → 1 token record (existing path).
    Stubs that retry deepseek fail (1); stubs that persist codex Err fail (2).</reality_test>
  <done_when>attempt_schedule for deepseek=1; persist-before-Err gated to DeepSeek typed
    failure with usage; regression-guard test for non-deepseek Err writes zero records.</done_when>
</task>

<task id="T-014" req="REQ-DS-020" wave="4" depends="T-012">
  <description>Stateless single-turn for DeepSeek. Runner returns session_id =
  "deepseek-<uuid_v4>"; inbound session_id is ignored (no resume). agent-worker's
  require_reused_worker treats DeepSeek's missing remote session as success. prewarm slot
  for "deepseek" is a safe no-op.</description>
  <files>daemon/crates/triumvirate/src/agent_exec.rs, daemon/crates/agent-worker/src/lib.rs, daemon/crates/mcp-bridge/src/deepseek.rs</files>
  <scope_out>Do NOT modify Gemini/Codex worker reuse semantics. Do NOT persist DeepSeek
    session_id as if it were a resume token. Do NOT add a remote-history layer in v1.</scope_out>
  <tools>cargo check, cargo test, uuid v4.</tools>
  <verify>cargo test -p triumvirate deepseek_stateless</verify>
  <reality_test>cargo test -p triumvirate deepseek_stateless — two sequential
    ask_agent {agent:"deepseek"} calls in the same cwd: both succeed; session_ids both
    start with "deepseek-" but are NOT equal; no remote-history was sent on the second call
    (verified via mock-server's last received request body); prewarm_daemon_workers does
    NOT spawn a deepseek worker (verified by absence of spawn record). A stub that errors
    on the second call (treating missing session as failure) fails.</reality_test>
  <done_when>Synthetic session_id format "deepseek-<uuid>"; require_reused_worker accepts
    missing remote session; prewarm slot is a safe no-op.</done_when>
</task>

<task id="T-015" req="REQ-DS-011,REQ-DS-012,REQ-DS-025" wave="4" depends="T-001">
  <description>Entry-point validation on the DeepSeek `ask_agent` path. (a) Anti-bulk
  byte-size intercept: when `agent=="deepseek"` AND `message.len() >
  TRIUMVIRATE_DEEPSEEK_BULK_BYTES` (default 16384), reject with a clear error mentioning
  "remote+metered" and "payload too large". (b) Confirm (via assertion / unit test) that
  this entry path performs NO auto-routing for `deepseek` (REQ-DS-011) and invokes NO
  sandbox initialization (REQ-DS-012) — the entry is consult/Q&A only.</description>
  <files>daemon/crates/triumvirate/src/agent_exec.rs</files>
  <scope_out>Do NOT apply the byte-size limit to Gemini or Codex (they're local CLIs — free
    on bulk). Do NOT bake the threshold in source (env-configurable).</scope_out>
  <tools>cargo check, cargo test.</tools>
  <verify>cargo test -p triumvirate deepseek_anti_bulk</verify>
  <reality_test>cargo test -p triumvirate deepseek_anti_bulk_and_constraints —
    (a) ask_agent {agent:"deepseek", message: "x".repeat(20000)} (20KB > default 16KB) →
        response is an explicit error containing the string "payload too large" and "metered";
    (b) ask_agent {agent:"gemini", message: same large string} → succeeds (no intercept);
    (c) override TRIUMVIRATE_DEEPSEEK_BULK_BYTES=32768 then 20KB deepseek call → succeeds.
    PLUS REQ-DS-011 / REQ-DS-012 constraint guards (same test, separate assertions):
    (d) grep -rE "auto.?route|router\\.route" daemon/crates/triumvirate/src/agent_exec.rs →
        0 hits for any deepseek-related auto-routing logic (REQ-DS-011);
    (e) grep -rn "sandbox" daemon/crates/mcp-bridge/src/deepseek*.rs → 0 hits
        (no sandbox invocation on the deepseek path — REQ-DS-012).
    Stubs that reject gemini bulk fail (b); stubs baking the threshold fail (c); a build that
    adds auto-routing or sandbox calls on the deepseek path fails (d)/(e).</reality_test>
  <done_when>Hard intercept only on agent=="deepseek"; threshold env-configurable;
    Gemini/Codex unaffected.</done_when>
</task>
```

---

## Wave 5 — Verification re-run + runbook

```xml
<task id="T-016" req="REQ-DS-017" wave="5" depends="T-000,T-001,T-002,T-003,T-004,T-005,T-006,T-007,T-008,T-009,T-010,T-011,T-012,T-013,T-014,T-015">
  <description>Run the Wave-0 probe battery END-TO-END against the funded account, in
  addition to the local mock-server tests, to ground-truth the full integration before
  ship. Capture the probe output as an audit artifact.</description>
  <files>daemon/docs/v1-deepseek/PROBE_RESULTS.md</files>
  <scope_out>Do NOT ship if any probe is RED. Do NOT skip the live-probe step even if all
    mock-server tests pass.</scope_out>
  <tools>cargo test -p triumvirate -- --ignored deepseek_contract; env vars set; output capture.</tools>
  <verify>cargo test -p triumvirate -- --ignored deepseek_contract</verify>
  <reality_test>The probe battery executes against api.deepseek.com with a real funded
    account and exits 0; PROBE_RESULTS.md contains the actual response bodies + HTTP codes
    for each probe (the audit trail). A stub run that doesn't hit the real API leaves an
    empty PROBE_RESULTS.md and fails this.</reality_test>
  <done_when>All probes green against the funded account; results captured for the audit.</done_when>
</task>

<task id="T-017" req="REQ-DS-018" wave="5" depends="">
  <description>Operator runbook: a markdown document mirroring `agy-operator-runbook.md`,
  covering: account-funding prerequisite + balance monitoring (GET /user/balance);
  full env-knob list (T-002); peak-hour 503 expectation (02:00-14:00 UTC) + the
  503=OpenTransient breaker behavior; system_fingerprint capture to per-request log;
  data-egress note (consult content goes to DeepSeek/China-routed); breaker tuning;
  the probe battery invocation; key-rotation procedure (the spec's API key in transcript
  must be rotated periodically).</description>
  <files>daemon/docs/deepseek-operator-runbook.md</files>
  <scope_out>Do NOT embed the API key. Do NOT invent runbook items not in the spec.</scope_out>
  <tools>file write only.</tools>
  <verify>test "$(grep -cE 'Account must be funded|GET /user/balance|env knob|peak-hour 503|system_fingerprint|data-egress|breaker tuning|probe battery|key rotation' daemon/docs/deepseek-operator-runbook.md)" -ge 9</verify>
  <reality_test>The doc covers all 9 required sections (funding, balance monitoring, env knobs,
    peak-hour 503, system_fingerprint, data-egress, breaker tuning, probe battery, key rotation)
    — verified by the grep above; a stub doc lacking funding or balance-monitoring instructions fails.</reality_test>
  <done_when>Runbook checked in at daemon/docs/deepseek-operator-runbook.md, structurally
    mirrors agy-operator-runbook.md.</done_when>
</task>
```

---

## Execution Contract

### Backlog Freeze
This document contains **18 tasks across 6 waves** (Wave 0 → Wave 5). This is the COMPLETE backlog.
- Do NOT accept new tasks until all tasks are complete (backlog_status: 0).
- If new requirements arrive mid-execution, respond: `blocked_on: scope-change — [describe new requirement]` and STOP.
- Only the human can add, remove, or reorder tasks in this backlog.

### Execution Order
- Wave order is STRICT: complete ALL tasks in Wave N before starting Wave N+1.
- Within a wave: tasks are parallel-safe (no dependencies on each other) UNLESS an explicit `depends` attribute says otherwise. Execute concurrently or in any order subject to depends.
- Within a sequential `depends` chain: strict FIFO. Do not start T(N+1) before T(N) is committed and reported.

### Definition of Done (Per Task)
A task is DONE when ALL of these are true:
1. Code is written (not stubbed — see reality test)
2. `<verify>` passes (compilation/type check)
3. `<reality_test>` passes (behavioral check that a stub cannot fake)
4. `<done_when>` condition is met (semantic completion check)
5. FULL test suite passes (`cargo test --workspace`) — not just this task's tests
6. Git commit is created with message referencing task ID
7. Format check passes (`cargo fmt --check` no diff)
8. Lint passes (`cargo clippy --workspace -- -D warnings` zero findings)
9. Type check passes (`cargo check --workspace`)
10. No new secrets in source (grep `TRIUMVIRATE_DEEPSEEK_API_KEY=sk-` returns zero hits)

A task that passes its own tests but breaks other tests is NOT done. Fix the regression first.

### Commit Report Format
After each task commit, respond with EXACTLY this format and nothing else:
```
task: T-{ID}
commit: {hash}
changed: {1-5 bullets, one per file or logical change}
tests: cargo test --workspace → {pass}/{total} passed
remaining: {N} tasks in current wave, {M} total
```
No interim progress updates. No explanations between tasks. No summaries until backlog_status: 0.

### Collateral Fix Protocol
If completing a task REQUIRES touching files outside that task's `<files>` list:
1. Label the commit: `collateral-fix: T-{ID} — {one-line justification}`
2. List extra files in the commit report under a `collateral:` field
3. Re-run full test suite after the collateral fix

If you WANT to touch adjacent code but don't NEED to, don't. Scope discipline > local improvement.

### Blocked Protocol
If blocked on any task, respond with EXACTLY:
```
blocked_on: {single concrete blocker}
task: T-{ID}
evidence: {command + output summary, max 5 lines}
proposed_fix: {single action you would take}
```
Then STOP.

### Context-Switch Refusal
If you receive instructions not in this backlog during execution:
- Respond: "Outside current execution contract. Backlog has {N} remaining tasks. Complete backlog first, or explicitly cancel it."
- Do NOT start the new work.

### End-of-Execution Report
When all tasks are complete:
```
backlog_status: 0 remaining
completed_tasks: [T-000, T-001, ..., T-017]
total_commits: {N}
collateral_fixes: {N} ({list if any})
test_suite: cargo test --workspace → {pass}/{total}
probe_battery: cargo test -- --ignored deepseek_contract → green
```
