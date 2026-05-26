# Round 1 Deltas — 2026-05-25

Effective spec = candidate spec (`deepseek-integration-spec.md`) + this delta. Source spec is
NOT edited mid-ceremony; the EDIT gate merges this after the Decision Ledger.

## User Calls
| ID | Decision | Choice | REQs touched |
|----|----------|--------|--------------|
| D-01 | Access path + API-key rule | **Cloud `deepseek-v4-pro`, API key — DeepSeek EXCEPTION to "subscriptions only, never API keys."** Rationale (user): no GPU to self-host v4-pro; DeepSeek has no subscription product; per-token cost is compelling for the diversity value. Self-host NOT pursued (no hardware) but the OpenAI-compat boundary is kept so a self-host swap stays possible later. | REQ-DS-002, REQ-DS-003 |
| D-02 | Integration shape | **Native Rust HTTP** against the OpenAI-compatible endpoint (research + both twins converged; native runner returns `ParsedAgentResult`, does NOT fork `execute_ask_agent`). | REQ-DS-004, REQ-DS-014 |
| D-03 | Role / seat | **True 4th participant** — consultable via `ask_agent` on ANY topic (reasoning + general + code review), exactly like `gemini`/`codex`. NOT a narrow reasoning-only seat. No sandbox in v1 (consult/Q&A, not file-writing). Joins the `/council` surface as a 4th voice. A file-writing `dispatch_deepseek` is a deferred expansion. | REQ-DS-005, REQ-DS-012, REQ-DS-022 |
| D-04 | Model selection | **v4-pro for v1** ("get used to it"); model is a **config knob** (`TRIUMVIRATE_DEEPSEEK_MODEL`, default `deepseek-v4-pro`) so **Pro↔Flash is a one-line tuning swap** once usage data exists. | REQ-DS-005 |
| D-05 | Spend cap | v1 relies on **prepaid balance + HTTP 402 hard breaker-trip + exact metered spend in the ledger** (+ low-balance alerts). Client-side cumulative-spend breaker DEFERRED (cheap pricing + prepaid balance = low runaway risk). | REQ-DS-010 |

## Auto-Resolves (clanker consensus / research fact)
- A-01: Model family corrected — **`deepseek-v4-pro` / `deepseek-v4-flash`**; legacy `deepseek-chat`/`deepseek-reasoner` + R1/V3.2 framing retire 2026-07-24. Reasoning is a **mode on current V4 models (both pro AND flash)** (`thinking:{type:enabled}`, `reasoning_effort:high|max`), not a separate model; v1 defaults to `deepseek-v4-pro`. (Research C + correction.) → REQ-DS-005. [D-AUD-09 fix]
- A-02: **`deepseek` = new TOP-LEVEL agent name**, NOT a `GeminiBackend`-style enum. Full first-class surface: `is_supported_agent_name` (lib.rs:37), `execute_ask_agent` error text (agent_exec.rs:247), `/status` supported_agents (main.rs:1874), session-spawn error (main.rs:2008), prewarm (agent_exec.rs:1291), `display_agent_name`, mcp-tools inter_agent. **Prewarm is implemented as a safe NO-OP for DeepSeek** (satisfies the generic sibling contract but does nothing — reconciles with A-11's stateless/no-prewarm; Gemini D-AUD2-F4). (Codex.) → REQ-DS-013, REQ-DS-016.
- A-03: Native runner returns **`ParsedAgentResult`**; add a `"deepseek"` arm in `run_named_agent_with_session_and_model`; do NOT fork `execute_ask_agent`. (Codex.) → REQ-DS-014.
- A-04: **token-economics needs NO schema change** for cache tiers. `cached_per_mtok` (storage.rs:45) + `cached_tokens` (lib.rs:43) + `attribution.rs:62` already price input/output/cached separately, keyed by exact `model` string. Map `prompt_cache_miss_tokens→input_tokens`, `prompt_cache_hit_tokens→cached_tokens`, completion→`output_tokens`; add price rows `deepseek-v4-pro` + `deepseek-v4-flash`. (Codex corrected research + Gemini.) → REQ-DS-009.
- A-04b: **Reasoning tokens must not be dropped.** `persist_daemon_token_record` writes `thinking_tokens: 0` (agent_exec.rs:185) and `attribution.rs` costing ignores `thinking_tokens` (prices only input/output/cached). DeepSeek bills `reasoning_content` as **output** tokens, so the DeepSeek runner MUST **fold reasoning/thinking tokens into `output_tokens`** before persistence (or "exact" underbills). (Codex D-AUD-04.) → REQ-DS-009.
- A-05: Resilience — **concurrency-cap + breaker + token-bucket MAP; reset-window cooldown does NOT** (no quota reset; prepaid balance + dynamic 429 pressure). New module `deepseek_resilience.rs` (don't generic-copy agy). DeepSeek's own cap is **account concurrency** (v4-pro 500). (Both twins.) → REQ-DS-006.
- A-06: Failure classification — **402 = HARD breaker trip (manual reset / top-up)**, distinct from **429 = transient w/ `Retry-After`**; 5xx/timeout transient/thresholded. No granular ratelimit headers → self-manage concurrency. (Both.) → REQ-DS-006. *Acceptance (Gemini D-AUD2-F1):* a 402 response MUST instantly trip the breaker (open, no auto-retry), block further requests, and surface to the orchestrator; test asserts 402 does NOT enter the generic transient-retry path (no retry loop on depleted balance). 429 follows `Retry-After` backoff.
- A-07: **REQ-DS-007 rewritten** from "SIGKILL-backed timeout" to: "HTTP request has a cancellation-safe timeout policy — idle timeout RESET on each received chunk/keep-alive (DeepSeek may hold the connection ~10 min before inference), an absolute request ceiling, and dropping the future aborts the connection." SIGKILL is meaningless for native HTTP. **Requires STREAMING (`stream:true`)** so keep-alive comments and body chunks are observable to reset the idle timer (a plain non-streaming `.json().await` cannot observe keep-alive — Codex D-AUD-03). (Both.) → REQ-DS-007, REQ-DS-019.
- A-08: **Fail-loud is the default**; no silent sibling substitution (violates both "no silent failure" and the diversity purpose). If a degraded route is ever added it MUST set `answered_by_agent`/`answered_by_backend`/`degradation_reason` (shared-types). (Both.) → REQ-DS-008. *Acceptance (Gemini D-AUD2-F2):* a DeepSeek failure (402/breaker-open/timeout) bubbles up to the orchestrator as an explicit error; test asserts NO silent failover to another sibling.
- A-09: **Explicit-route only** — no auto-router exists today; Claude picks who to consult. DeepSeek extends `/council` to a 4-way. (Both.) → REQ-DS-011.
- A-10: **No sandbox in v1** — `ask_agent` is consult/Q&A, no file writes. (Both.) → REQ-DS-012.
- A-11: **v1 stateless single-turn** — worker state stores one `session_id` per agent::cwd; DeepSeek HTTP has no honest resume. session_id is synthetic-for-accounting only; do NOT promise per-cwd continuity; handle `require_reused_worker` missing-session semantics; **prewarm deferred/disabled** for a metered stateless API. (Codex.) → REQ-DS-020 (new).
- A-12: **Streaming consumption + lifecycle events** — v1 streams the response (per A-07) and emits lifecycle events (request-started / keepalive-observed / first-token / response-received / usage-parsed) so `events_tx` isn't just generic timer heartbeats. `delta.reasoning_content` is parsed SEPARATELY — chain-of-thought NEVER goes into `.response`. (Codex.) → REQ-DS-019.
- A-13: **v1 env knobs (required):** `TRIUMVIRATE_DEEPSEEK_BASE_URL` (default `https://api.deepseek.com/v1`), `TRIUMVIRATE_DEEPSEEK_API_KEY` (NEVER logged, NEVER in argv, used only as `Authorization: Bearer`), `TRIUMVIRATE_DEEPSEEK_MODEL` (default `deepseek-v4-pro`), request-timeout, concurrency-cap, rate knobs. **Deferred (NOT v1):** client-side hard spend-cap envs (low-balance alert knobs are separate). `BIN`/`ARGS` only behind an explicit deferred CLI backend. (Codex D-AUD-02/07/08.) → REQ-DS-015.
- A-14: **Spend-cap mechanism gap** — `persist_daemon_token_record` writes `cost_usd: None` (agent_exec.rs:187); attribution computes cost LATER, so any pre/at-dispatch cap needs a **synchronous cost calc** using the same pricing lookup, not the delayed `cost_usd`. (Codex.) → REQ-DS-021 (new; deferred per D-05 but recorded).
- A-15: **OpenAI-compat nuances to honor:** target `/v1`; `n=1` only; no `logprobs`; `response_format json_object` requires "json" in the prompt (else 400); strip `reasoning_content` from any resent history (moot — v1 single-turn). (Research A/B + Gemini.) → REQ-DS-004 notes.

## REQ Additions / Renames / Deletions
- ADD **REQ-DS-019** — `events_tx` lifecycle (STREAMING). *Done when:* the DeepSeek runner consumes a streamed response and emits ordered events `request_started`, `keepalive_observed` (on any keep-alive comment/empty chunk), `first_token`, `response_received`, `usage_parsed`; and `delta.reasoning_content` is accumulated SEPARATELY and never appears in `.response`. *Test:* a mocked stream with a keep-alive chunk + reasoning chunk + content chunk yields the 5 events and a `.response` containing only `content`.
- ADD **REQ-DS-020** — v1 stateless single-turn. *Done when:* DeepSeek is EXCLUDED from `prewarm_daemon_workers()`; `session_id` returned is a synthetic accounting id (documented format, e.g. `deepseek-<uuid>`) with NO resume reuse; `require_reused_worker` does not treat DeepSeek's missing remote session as a failure. *Test:* two sequential `ask_agent deepseek` calls do not error on session reuse and share no conversational state.
- ADD **REQ-DS-021** — client-side spend-cap mechanism (DEFERRED for v1 per D-05). *Recorded constraint:* a future cap MUST compute projected/actual cost synchronously via the pricing lookup (NOT the persisted `cost_usd`, which is `None` at persist time — agent_exec.rs:187). No v1 acceptance test (deferred).
- ADD **REQ-DS-022** — "true 4th participant." *v1 (this spec, daemon):* `deepseek` is consultable via `ask_agent` on ANY topic (reasoning/general/coding answers) as a full first-class agent. *Done when:* `ask_agent {agent:"deepseek"}` succeeds for a reasoning, a general, and a code-review prompt. *Follow-on (NOT a daemon REQ):* the `/council` SKILL (Claude-side, not daemon code) gains `deepseek` as a 4th participant — tracked separately. *Deferred:* file-writing `dispatch_deepseek`.
- REWRITE **REQ-DS-007** — streaming, keep-alive-aware cancellation-safe HTTP timeout (see A-07), replacing SIGKILL framing. *Done when:* idle timer resets on each chunk; an absolute ceiling aborts a truly hung request; a dropped future closes the connection.
- AMEND **REQ-DS-002/003** — cloud `deepseek-v4-pro` + ratified API-key EXCEPTION (D-01). The native HTTP path targets the OpenAI-compatible DeepSeek Cloud API directly (architectural necessity — no CLI-wrapper overhead). Self-host is NOT a roadmap commitment (user declined, no hardware); it merely remains *possible* later as a `base_url` config swap, given the standard API shape. (Gemini D-AUD2-F3.)
- AMEND **REQ-DS-005** — v4-pro default; model is a config knob (`TRIUMVIRATE_DEEPSEEK_MODEL`, Pro↔Flash tunable); reasoning is a mode param (`thinking`/`reasoning_effort`).
- AMEND **REQ-DS-009** — exact metering via existing schema + cache-tier mapping (miss→input, hit→cached) + price rows; **reasoning tokens folded into `output_tokens`** before persist (A-04b).
- AMEND **REQ-DS-014** — native HTTP adds the `"deepseek"` arm ONLY at `run_named_agent_with_session_and_model` (returning `ParsedAgentResult`); **`run_agent_process_with_session` is UNCHANGED** (it is subprocess-shaped — bin/args) unless a deferred CLI backend is built. Supersedes the candidate spec's wording that named both functions. (Codex D-AUD-01.)
- AMEND **REQ-DS-015** — v1 connector = native HTTP env knobs (A-13); the candidate spec's `deepseek_command()` + `TRIUMVIRATE_DEEPSEEK_BIN/_ARGS` + `resolve_connector_command` arm are **DEFERRED to a CLI backend, NOT v1**. (Codex D-AUD-02.)
- No deletions. No ID collisions (new IDs 019-022 verified unused vs candidate 001-018).

## Net State After Round 1
- REQ count: **22** (REQ-DS-001…022; was 18, +4: 019/020/021/022).
- Auto-resolves total: 15 (A-01…A-15).
- User decisions total: 5 (D-01…D-05).
- New REQs added this round: REQ-DS-019, 020, 021, 022.
- Critical defects caught this round:
  1. "Flash = non-reasoning" (my error) — corrected by user challenge + live data; had contaminated the twin review's D-01 reasoning. Flash is a near-peer reasoner (−1..−3 pts).
  2. token-economics schema-change assumption (Gemini/research) — corrected by Codex reading code: no schema change needed.
  3. spend-cap can't read delayed `cost_usd` (Codex) — needs synchronous calc.
  4. stateless session semantics + prewarm for a metered HTTP agent (Codex).
  5. SIGKILL meaningless for native HTTP; 10-min keep-alive would kill legit requests (both).

## Open for Round 2 (light — convergence reached)
- Verify the corrected premise (Flash reasoning, cloud v4-pro, true-4th-participant) under a clean twin pass (the R1 twin review ran on the false "Flash non-reasoning" premise for D-01).
- Confirm REQ-DS-019/020/021/022 wording + acceptance criteria.
- REQ-DS-017 verification probe battery scope against the real cloud endpoint.

## Twin Audit (R1.8 — both twins, fresh/blind)
**Auditors:** `deepseek-delta1-codex` (daemon, fresh turns:0) + Gemini (via `gemini` MCP pro/high
— daemon session path was down; CLI/MCP fallback per skill). Both reviewed independently.

**Codex (code-grounded) — 9 findings, ALL FIXED:**
- D-AUD-01 (High) → AMEND REQ-DS-014: deepseek arm only at `run_named_…`, `run_agent_process_with_session` unchanged. ✅
- D-AUD-02 (High) → AMEND REQ-DS-015: native HTTP env knobs; command/BIN/ARGS deferred. ✅
- D-AUD-03 (High) → A-07/REQ-DS-007/019: require STREAMING so keep-alive is observable. ✅
- D-AUD-04 (Med) → A-04b: fold reasoning tokens into output_tokens (else underbill). ✅
- D-AUD-05 (Med) → REQ-DS-022 split: v1 ask_agent (daemon) vs council (skill follow-on). ✅
- D-AUD-06 (Med) → done-when + test added to REQ-DS-019/020/022. ✅
- D-AUD-07 (Med) → `TRIUMVIRATE_DEEPSEEK_API_KEY` named; never logged/argv; Bearer only. ✅
- D-AUD-08 (Low) → A-13 envs split v1-required vs deferred spend-cap. ✅
- D-AUD-09 (Low) → A-01 reworded (reasoning is a mode on both pro & flash). ✅
- Codex CLEAN checks: A-03 (ParsedAgentResult), A-04 (no schema change), A-11 (prewarm), A-14
  (cost_usd None), REQ-ID hygiene, Flash-residue — all verified clean against the real code.

**Gemini (coherence) — 4 findings, ALL FIXED:**
- F1 (Critical) → A-06 acceptance: 402 instantly trips breaker, no transient-retry loop. ✅
- F2 (Med) → A-08 acceptance: failure bubbles up loud, no silent failover. ✅
- F3 (Low) → REQ-DS-002/003 reworded: OpenAI-compat = architectural necessity; self-host not a roadmap commitment. ✅
- F4 (Low) → A-02: prewarm implemented as a safe NO-OP for DeepSeek (reconciles with A-11). ✅
- Gemini CLEAN: Flash-premise residue (none), API-key-exception rationale (coherent).

**Verdict: R1.8 PASS** — both twins' defects resolved in this revised delta. No open criticals/highs.
Self-audit note: the "Flash non-reasoning" error was Claude's (caught by the user, not the
twins, who had inherited it) — recorded as the round's top defect for the postrodeo.
