# Round 3 — Challenge-class research (the "what bites you" category)

Prompted by the operator's question: do we understand the CHALLENGES, not just the mechanics?
Answer was "partially" — we had API mechanics + code-derived internal friction, but NOT
lessons-learned/pitfalls research. This fills it. Several findings CHANGE the spec.

## CRITICAL — corrects an R2 decision
### C-1: `max_tokens` is a SHARED reasoning+answer budget → tight cap = EMPTY answer
- Reasoning models spend `max_tokens` on BOTH the hidden reasoning AND the final answer. If
  reasoning consumes the budget, `content` comes back **empty** (only `reasoning_content`).
- **This breaks R2-A06/D-07's "max_tokens default 4096 as the frugality lever."** With thinking
  ON, a hard question can burn 4000 reasoning tokens and return NO answer.
- **Fix:** max_tokens must be GENEROUS (3-5× the expected answer; e.g. default **16384**), and is
  NOT the frugality throttle. Frugality comes from thinking on/off + reasoning_effort + an
  in-flight reasoning-token sanity cap (C-2), NOT a tight max_tokens. (Reverses my earlier claim.)

### C-2: Reasoning "deliberation loops" → runaway cost (50k thinking tokens on a simple task)
- Reasoning models can loop ("I haven't thought enough yet"), burning tens of thousands of
  reasoning tokens on trivial prompts. This is the REAL frugality risk (not output length).
- **Fix:** in-flight reasoning-token monitoring — count streamed `reasoning_content`; if it exceeds
  a sanity threshold (knob, e.g. ~8-10k for a routine consult), abort the stream and fail loud
  (or surface, optionally retry effort-lower). NEW frugality control.

## HIGH — new failure modes our error model missed
### C-3: "Ghost Success" — mid-stream errors arrive as HTTP 200 with an error object IN the stream
- The 200 OK header is sent when the stream opens; a later failure can't un-send it. Providers/
  gateways emit `data: {"error":{"message":...,"code":...}}` INSIDE the SSE stream.
- **Our R1/R2 error taxonomy assumed HTTP status codes — incomplete.** A 200 stream can still fail.
- **Fix:** the SSE parser MUST inspect every chunk for an `error` key and trigger the SAME error
  classification/breaker path as an HTTP error, even though the network said 200. NEW requirement.

### C-4: `finish_reason` blind spot — stream end ≠ success
- A stream can end because it was KILLED: `finish_reason:"length"` (truncated — incomplete answer),
  `"content_filter"`, or null/unknown.
- **Fix:** check `finish_reason` before treating a consult as complete. `length` → the answer is
  truncated (surface/flag, don't present as whole; with generous max_tokens from C-1 this is rare);
  `content_filter`/null → flag. NEW requirement.

## MEDIUM
### C-5: Logging/event firehose — don't emit per-token
- Per-chunk logging/events at scale = cost + collapse. **Fix:** events_tx stays LOW-NOISE
  (lifecycle + keep-alive + periodic, NOT per-token); persist the final reconstructed message once.
  Refines REQ-DS-019.
### C-6: DeepSeek reliability is real — outages + peak-hour 503/429
- 2026 ops reality: a 7h blackout (Mar 29-30), an API outage (Apr 22), May volatility. **Peak
  (02:00-14:00 UTC)** → frequent 503-overloaded + 429 even with balance; **off-peak (16:30-00:30
  UTC)** stable. 503s are returned immediately (not queued) → aggressive backoff + breaker. TTFT
  can spike >10s under load. Community verdict: "best SECONDARY you can't rely on as primary."
- **Implication (validates our design):** DeepSeek WILL be down sometimes. Our **fail-loud +
  breaker + consult-seat (not primary)** posture is correct — a failed consult just fails; Claude/
  user retries or asks another sibling explicitly (NO silent substitution). Note for the runbook:
  expect peak-hour 503s; the breaker should absorb them.
### C-7: Quality shifts under load (suspected quantized/distilled routing) + China routing
- Under load, reports of faster-but-garbage output (suspected fallback to distilled models) and
  latency from China-routed infra. **Fix:** capture `system_fingerprint` for observability (detect
  backend shifts); note in the runbook that consult CONTENT goes to DeepSeek's servers (data-egress
  reality of the API-key path — already accepted, but document it).

## Validated (no change)
- 2-5 min latency for high-effort reasoning → our generous absolute ceiling (default 600s) + 60s
  idle timeout + streaming is the right shape (NOT sync request/response).
- Single-turn (REQ-DS-020) AVOIDS the "400 on stripped reasoning_content in history" class entirely.
- reasoning_content billed as output → fold into output_tokens (A-04b) confirmed.

## Net new spec impact (→ Round 3 delta)
- REVERSE R2-A06: max_tokens generous (default 16384), NOT the frugality lever. → REQ-DS-005.
- NEW: in-flight reasoning-token sanity cap (abort runaway thinking). → new REQ.
- NEW: SSE parser checks each chunk for `error` (ghost-success) + checks `finish_reason`. → new REQ / REQ-DS-006/019.
- REFINE: events_tx low-noise (no per-token). → REQ-DS-019.
- REFINE: runbook notes peak-hour 503s, system_fingerprint observability, data-egress. → REQ-DS-018.
