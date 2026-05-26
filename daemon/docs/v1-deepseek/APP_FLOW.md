# APP_FLOW — DeepSeek consult flows

> Every user-visible hop is defined. Every failure surfaces loud. No silent substitution.

## Primary flow: a successful consult

```
operator (Claude Code chat)
  │  "ask deepseek about <X>"
  ▼
Claude (orchestrator)
  │  mcp__triumvirate__ask_agent {agent:"deepseek", message:"<X>", deepseek_thinking?, deepseek_effort?, …}
  ▼
Triumvirate daemon — HTTP POST /ask-agent
  ├─ is_supported_agent_name("deepseek") = true                   [REQ-DS-013]
  ├─ anti-bulk byte-size intercept (≤16KB by default)              [REQ-DS-025]
  ├─ acquire worker (stateless; synthetic session_id)              [REQ-DS-020]
  └─ execute_ask_agent: attempt_schedule = ONE outer attempt       [REQ-DS-006]
        ▼
   DeepSeek runner (mcp-bridge::deepseek)
     ├─ acquire semaphore permit + token-bucket permit             [REQ-DS-006]
     ├─ check breaker state (closed / OpenTransient / Hard-…)       [REQ-DS-006]
     ├─ reqwest::Client builder: read_timeout 60s, timeout 1800s,
     │   tcp_keepalive 30s                                          [REQ-DS-007]
     └─ POST https://api.deepseek.com/v1/chat/completions
          Authorization: Bearer $TRIUMVIRATE_DEEPSEEK_API_KEY
          Content-Type:  application/json
          { model: deepseek-v4-pro, messages: […], stream: true,
            stream_options: {include_usage: true},
            thinking: {type:"enabled"}, reasoning_effort: "high",
            max_tokens: 32768 }                                     [REQ-DS-015]
              ▼
   SSE stream from api.deepseek.com
     ├─ ":" keep-alive comments → throttled progress event (≤1/30s)  [REQ-DS-019]
     ├─ data: {…delta.reasoning_content…} chunks (N×)               [REQ-DS-019/023]
     ├─ data: {…delta.content…} chunks (N×)
     ├─ per-chunk: scan for top-level "error" (ghost-success guard) [REQ-DS-029]
     ├─ data: {choices:[], usage:{…}}  ← include_usage chunk         [REQ-DS-009]
     └─ data: [DONE]                                                 [REQ-DS-019]
              ▼
   runner.finalize:
     ├─ finish_reason check: stop = ok; length/content_filter/null
     │   = LOUD FAIL (no partial gibberish to Claude)                [REQ-DS-030]
     ├─ usage mapping: output ← completion_tokens (incl reasoning);
     │   input ← prompt_cache_miss_tokens; cached ← prompt_cache_hit  [REQ-DS-009]
     ├─ synchronous cost calc from price_table (v4-pro/flash row)    [REQ-DS-021]
     └─ CoT bifurcation: .response = content ONLY; reasoning_content
         persisted to per-request log; include_reasoning=true opts in  [REQ-DS-023]
              ▼
   ParsedAgentResult → execute_ask_agent → AskAgentResponse →
   MCP response → Claude → operator sees the answer + cost line.
```

**Visible lifecycle events** (REQ-DS-019, no timing promises — actual latency depends on
prompt + queue + reasoning_effort): `request_started` → optional `keepalive_observed` while
queued → `first_reasoning` → `first_answer_token` → `usage_parsed` → `completion`. On
completion the daemon emits a per-consult cost line (e.g. `cost: $0.0006 — in 0/18 (hit/miss),
out 174 (120 reasoning)`).

## Failure flows (all LOUD)

```
402 Insufficient Balance  → HardOpenInsufficientBalance (no auto-recovery)
                            → operator gets a clear "fund the account" error
                            → no retries until manual top-up                          [REQ-DS-006/024]
401 bad key               → fail loud; runbook directs to env audit                   [REQ-DS-006]
400/422 bad request       → fail loud, no retry, surface error.message               [REQ-DS-006]
429 rate limit            → bounded internal retry honoring Retry-After; if it
                            recurs, OpenTransient (threshold+cooldown)                [REQ-DS-006]
5xx/503 overload          → bounded retry/backoff; thresholded OpenTransient          [REQ-DS-006]
idle timeout (default 60s)→ "DeepSeek: no bytes for <READ_TIMEOUT_SECS>s, aborting" — loud  [REQ-DS-007/024]
absolute ceiling (def 1800s)→ "DeepSeek: consult exceeded <TIMEOUT_SECS>s ceiling — fail loud
                            (raise ceiling or lower reasoning_effort)"                      [REQ-DS-007/024]
runaway-reasoning abort   → "DeepSeek: reasoning ran past <CAP> tokens — aborted
                            locally" (only if env knob set; default disabled)         [REQ-DS-028]
mid-stream embedded error → classified as if HTTP status; same breaker policy         [REQ-DS-029]
finish_reason:length      → "DeepSeek: response truncated (length) — raise
                            max_tokens or simplify"                                   [REQ-DS-030]
mid-stream dirty disconnect → loud + estimated token record                          [REQ-DS-026]
breaker open              → "DeepSeek unavailable — circuit open until <reason>"
                            (no silent sibling substitution)                          [REQ-DS-008]
```

**On every failure path that touched paid traffic, a token record is persisted** (exact if the
usage chunk arrived, else estimated) — DeepSeek charges either way (REQ-DS-026).

## Operator monitoring flow

```
operator: triumvirate daemon /status                  → supported_agents includes "deepseek"
operator: tail token-economics ledger                 → DeepSeek rows with usage_source ∈ {exact,estimated}
operator: GET https://api.deepseek.com/user/balance   → monitor topped_up_balance
operator: peak-hour 503 spike (02:00-14:00 UTC)       → expected per runbook; breaker absorbs
operator: backend shift suspicion                     → grep DeepSeek log records for system_fingerprint deltas
```
