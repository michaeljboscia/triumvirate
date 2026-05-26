# PRD — DeepSeek 4th-sibling integration (v1)

> Source spec: `daemon/docs/specs/deepseek-integration-spec.md` (canonical, 30 REQ-IDs).
> Every FEAT below maps to one or more REQ-DS-### and is covered by a TEST_PLAN row.

## Product summary
Add a fourth Triumvirate sibling — **`deepseek`** — that the operator's Claude session can
consult via the existing `ask_agent` MCP tool, exactly like `gemini` and `codex`. It is the
first **non-Anthropic/Google/OpenAI** voice (genuinely independent dissent) and the first
**API-backed** (not CLI-subprocess) sibling in the daemon.

## Features

### FEAT-001 — `deepseek` is a first-class agent
*Maps to:* REQ-DS-001, REQ-DS-013, REQ-DS-016, REQ-DS-022.
**Acceptance:**
- `mcp__triumvirate__ask_agent {agent:"deepseek", message:"..."}` is accepted by the daemon
  (`is_supported_agent_name` returns true; the request is not rejected as an unknown agent).
- `daemon /status` lists `deepseek` in `supported_agents` alongside `gemini` and `codex`.
- `display_agent_name("deepseek")` returns a sensible display name.
- The operator can ask DeepSeek about reasoning, general, AND code-review questions — done-when
  test: three sample prompts (one of each kind) succeed end-to-end.

### FEAT-002 — Native HTTP runner with streaming SSE
*Maps to:* REQ-DS-002, REQ-DS-003, REQ-DS-004, REQ-DS-014, REQ-DS-015, REQ-DS-019, REQ-DS-023.
**Acceptance:**
- A hand-rolled `reqwest` client streams `POST /v1/chat/completions` against
  `api.deepseek.com/v1` using `Authorization: Bearer $TRIUMVIRATE_DEEPSEEK_API_KEY` and
  `Content-Type: application/json`.
- The runner parses SSE chunks: ignores keep-alive comments (`:` lines), recognizes
  `data: [DONE]`, accumulates `delta.reasoning_content` separately from `delta.content`.
- The runner returns `ParsedAgentResult` (no fork of `execute_ask_agent`).
- CoT bifurcation default: `.response` contains FINAL `content` only; `reasoning_content` is
  stored to a per-request log/ledger record (NOT `OutboxEvent.detail`).
- A per-call `deepseek_include_reasoning=true` flag opts in to receiving the trace.

### FEAT-003 — DeepSeek-owned resilience
*Maps to:* REQ-DS-006, REQ-DS-008, REQ-DS-024, REQ-DS-029, REQ-DS-030.
**Acceptance:**
- New `deepseek_resilience.rs` provides `tokio::sync::Semaphore` (default cap 8), token bucket
  (default 60 RPM), and a breaker with three states: `HardOpenInsufficientBalance` (no
  auto-half-open), `OpenTransient` (backoff+cooldown), `HalfOpen`.
- `execute_ask_agent`'s `attempt_schedule` for `agent=="deepseek"` is **ONE outer attempt**
  (the generic 3-attempt loop at ~agent_exec.rs:373 is bypassed for DeepSeek).
- Classification keys on **HTTP status**: 400/401/402/422 → never retry (402 → hard breaker);
  429 honors `Retry-After`; 5xx/503 → transient; pre-first-byte connection failure → 1
  internal retry; mid-stream dirty disconnect → fail loud.
- Stream-embedded errors (a `data: {"error":...}` chunk inside an HTTP-200 response) are
  detected per-chunk and routed through the same classification path.
- `finish_reason ∈ {length, content_filter, null}` → runner returns a hard error (not
  `Ok(parsed)`, no partial gibberish to Claude); provider breaker NOT tripped.
- All failure modes are **fail-loud** — no silent substitution to another sibling.

### FEAT-004 — Two-timer cancellation-safe timeout
*Maps to:* REQ-DS-007, REQ-DS-024.
**Acceptance:**
- **Idle/read-timeout 60s** (rolling, reset by any received byte incl. keep-alive comments) is
  the primary dead-stream detector — fires loud on a hung stream.
- **Absolute SLA ceiling 1800s** is a generous backstop (outer `tokio::time::timeout` around
  the attempt), env-configurable.
- `tcp_keepalive` set to 30s.
- Error messages distinguish idle-vs-ceiling-vs-runaway-vs-provider; no hardcoded "60s" text.

### FEAT-005 — Frugality posture + per-call override surface
*Maps to:* REQ-DS-005, REQ-DS-027, REQ-DS-028.
**Acceptance:**
- Default request body carries `thinking:{type:"enabled"}` + `reasoning_effort:"high"` (the
  reasoning-diversity value).
- `max_tokens` defaults to **32768** (generous, shared budget — empirically required to avoid
  empty-answer starvation).
- `AskAgentRequest` gains OPTIONAL fields `deepseek_thinking`, `deepseek_reasoning_effort`,
  `deepseek_include_reasoning`, `deepseek_max_tokens` — defaulting to `None` ⇒ env defaults.
  Backward-compatible (Gemini/Codex ignore).
- The `thinking` toggle is **the frugality lever**: Claude can pass `deepseek_thinking:"disabled"`
  per-call for cheap/quick consults.
- Optional `_REASONING_CAP_TOKENS` runaway breaker (default 0=disabled; when set, validated
  `< _MAX_TOKENS`); aborts locally without tripping the provider breaker.

### FEAT-006 — Metering: exact-with-cache-tiers
*Maps to:* REQ-DS-009, REQ-DS-010, REQ-DS-021, REQ-DS-026.
**Acceptance:**
- `token-economics` `price_table` gains rows for `deepseek-v4-pro` (in-miss $0.435, in-hit
  $0.003625, out $0.87) and `deepseek-v4-flash` ($0.14 / $0.0028 / $0.28). **No schema change.**
- Usage mapping (empirically verified):
  - `output_tokens ← completion_tokens` (already INCLUDES `completion_tokens_details.reasoning_tokens` — no double-add).
  - `input_tokens ← prompt_cache_miss_tokens`.
  - `cached_tokens ← prompt_cache_hit_tokens`.
- Metering source = **usage-chunk-received → exact**; **else → estimated** (`bytes_received/4 + prompt_estimate`).
- ALL paid paths persist a token record — including dirty disconnect, reasoning-cap abort,
  ghost-success, bad `finish_reason`, ceiling/idle timeout. (`execute_ask_agent`'s `Ok`-only
  persistence at ~581 is extended via the runner's typed-failure-with-usage; the persist-before-`Err`
  path is **gated to DeepSeek's typed failure only** — Gemini/Codex/Claude `Err` paths unchanged.)
- Per-consult cost is **visible in v1**: a lifecycle/progress cost line synchronously computed
  from the price table after the usage chunk. (Cumulative hard spend-cap deferred.)

### FEAT-007 — Anti-bulk hard intercept
*Maps to:* REQ-DS-025.
**Acceptance:**
- The daemon's DeepSeek `ask_agent` path rejects any payload exceeding a configurable byte
  threshold (default ~16KB) with a clear error: "DeepSeek is remote+metered — payload too
  large; use a local sibling for bulk data." Plus a soft global note in the tool description.

### FEAT-008 — Wave-0 verification probe battery
*Maps to:* REQ-DS-017.
**Acceptance:**
- A `cargo test -- --ignored` suite at `daemon/crates/triumvirate/tests/deepseek_contract.rs`
  (or equivalent), env-gated on `TRIUMVIRATE_DEEPSEEK_API_KEY`, ground-truths the live API
  before any production wiring is enabled. **Prerequisite:** a funded DeepSeek account.
- Probes: auth ok, model resolves (v4-pro and v4-flash), streaming parses (reasoning_content
  separated, `[DONE]` seen, usage chunk present with `prompt_cache_hit_tokens` /
  `prompt_cache_miss_tokens` / `completion_tokens_details.reasoning_tokens`), 401 on bad key,
  422 on a malformed request, `finish_reason:"length"` reproducible by setting tiny max_tokens.

### FEAT-009 — Operator runbook
*Maps to:* REQ-DS-018.
**Acceptance:**
- A `daemon/docs/deepseek-operator-runbook.md` mirroring `agy-operator-runbook.md` documents:
  account-funding prerequisite, balance-monitoring (`GET /user/balance`), peak-hour 503
  expectation (02:00-14:00 UTC), `system_fingerprint` capture to a per-request log record,
  data-egress note (consult content goes to DeepSeek/China-routed), env knobs, breaker tuning.

## Stateless contract (constraint applied to FEAT-002)
*Maps to:* REQ-DS-020.
- v1 is stateless single-turn. The runner returns a synthetic `session_id`
  (`deepseek-<uuid>`), does NOT support resume, and is **excluded from prewarm** (prewarm slot
  for `deepseek` is a safe no-op). `require_reused_worker` treats DeepSeek's missing remote
  session as success.

## Routing & sandbox (constraints, no separate feature)
*Maps to:* REQ-DS-011, REQ-DS-012.
- Explicit-route only (Claude decides per-call). No auto-router. Council integration is a
  deferred Claude-side skill follow-on, NOT v1 daemon scope.
- No sandbox in v1 (`ask_agent` is consult/Q&A, not file-writing).

## Deferred (Section 7 of source spec)
- File-writing `dispatch_deepseek` (requires a sandbox).
- `/council` skill 4-way integration (Claude-side, not daemon).
- Cumulative hard spend-cap (visibility-only in v1 — REQ-DS-021).
- CLI-subprocess backend (`_BIN`/`_ARGS` env shape).
- Self-host of v4-pro / v4-flash (no GPU).
- Auto-routing among siblings (no router exists).
