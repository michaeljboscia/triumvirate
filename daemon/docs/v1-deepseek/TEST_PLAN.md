# TEST_PLAN — DeepSeek 4th-sibling integration (v1)

> Every REQ-DS-### gets at least one row with a behavioral **Reality Test** that a stub
> cannot pass. Tests live in the crates indicated in BACKEND_STRUCTURE.md.

## Identity & first-class surface

| REQ | Acceptance criteria | Type | Pass condition | Reality test (stub-proof) | Pre-impl baseline |
|---|---|---|---|---|---|
| **REQ-DS-001** | `ask_agent {agent:"deepseek"}` accepted; uses existing tool | Integration | request reaches the deepseek arm; no "unknown agent" error | `ask_agent` to a known mock returns Ok with `.response` carrying the mock's content (a stub returning the same string for ALL agents fails: assert each agent's response differs given different mocks) | today: `ask_agent {agent:"deepseek"}` returns "unsupported agent" |
| **REQ-DS-013** | `is_supported_agent_name("deepseek")==true` | Unit | assertion in mcp-bridge | A two-call test: `is_supported_agent_name("deepseek")==true` AND `is_supported_agent_name("claude")==false` — a stub returning `true` for all names fails the second assertion | today: only gemini/codex true |
| **REQ-DS-016** | Surface complete: display name, /status, error texts, prewarm, mcp-tools | Integration | display="DeepSeek"; /status includes "deepseek"; session-spawn error mentions "deepseek" for an unknown agent variant | HTTP GET /status returns JSON whose `supported_agents` array literally contains "deepseek" (a stub list-of-strings that just echoes the gate's input fails because /status reads its own list) | today: gemini/codex only |
| **REQ-DS-022** | Consultable on reasoning/general/code-review prompts | Integration | three sample prompts return non-empty, sensibly-different content | Send three semantically distinct prompts to the mock; assert three distinct `.response` strings (a stub returning a fixed string fails) | n/a |

## Wiring (dispatch path)

| REQ | Acceptance | Type | Pass condition | Reality test | Baseline |
|---|---|---|---|---|---|
| **REQ-DS-002** | Access path = cloud api.deepseek.com/v1 | Unit + manual | DeepSeekConfig::default base_url is api.deepseek.com/v1 | Reading `TRIUMVIRATE_DEEPSEEK_BASE_URL` after env-set to `http://localhost:9999` returns that override (stub: hardcoded URL fails) | n/a |
| **REQ-DS-003** | API-key exception; key never logged/argv | Unit | Debug for the API-key wrapper redacts | `format!("{:?}", cfg)` does NOT contain the secret string (stub Display that prints the secret fails) | n/a |
| **REQ-DS-004** | Hand-rolled reqwest + SSE (not async-openai) | Code-review + cargo tree | `cargo tree -p mcp-bridge \| grep async-openai` is empty | grep returns 0 lines; reqwest is referenced from `src/deepseek.rs` (a stub introducing async-openai fails the cargo-tree assertion) | n/a |
| **REQ-DS-014** | `"deepseek"` arm in `run_named_…`; ParsedAgentResult; no fork of `execute_ask_agent` | Integration | dispatch reaches the arm; ParsedAgentResult returned | mock-server test from T-010: `run_named_…(agent="deepseek", …)` returns `Ok(ParsedAgentResult{response:"mock",…})`; ALSO assert `execute_ask_agent` source has no `match agent { "deepseek" => … }` fork (grep) | today: no arm |
| **REQ-DS-015** | All 15 env knobs with defaults; API_KEY redacted | Unit | DeepSeekConfig::from_env() matches REQ-DS-015 defaults | with all env unset (except API_KEY), assert: MAX_TOKENS=32768, TIMEOUT_SECS=1800, READ_TIMEOUT_SECS=60, MAX_CONCURRENT=8, MAX_RPM=60, REASONING_CAP_TOKENS=0, LOG_DIR ends with "deepseek-logs/", LOG_REASONING_CAP_BYTES=262144, BULK_BYTES=16384 (a stub returning all zeros fails) | n/a |

## Models, thinking, frugality

| REQ | Acceptance | Type | Pass condition | Reality test | Baseline |
|---|---|---|---|---|---|
| **REQ-DS-005** | v4-pro default; Pro↔Flash via env; thinking+effort as body fields | Integration | DeepSeekConfig.model defaults to "deepseek-v4-pro"; the JSON body posted to the mock contains `thinking:{type:"enabled"}` and `reasoning_effort:"high"` and `model:"deepseek-v4-pro"` | mock asserts the EXACT body JSON received (a stub posting empty `{}` fails) | n/a |
| **REQ-DS-027** | Optional per-call override fields backward-compat | Unit + integration | serde round-trip preserves values; gemini/codex requests parse without errors | as in T-011 reality test: round-trip with disabled + max_tokens=512 preserves both; gemini request with no deepseek fields parses successfully (stub that requires the fields fails) | n/a |
| **REQ-DS-028** | Optional runaway-reasoning early-abort, default disabled | Unit | cfg with cap=0 → no abort on any input; cap=100 → abort after ~100-token-equivalent | as in T-008 reality test (1000-char reasoning aborts at cap=100; same input completes at cap=0; breaker remains Closed in both cases) | n/a |

## Streaming, lifecycle, CoT bifurcation

| REQ | Acceptance | Type | Pass condition | Reality test | Baseline |
|---|---|---|---|---|---|
| **REQ-DS-019** | Streaming SSE + low-noise lifecycle events; ignore keep-alive; reach [DONE] | Unit (parser) + integration | parser yields 5 lifecycle events in the right order on a canned stream; <=1 keep-alive event per 30s | the parser test from T-006: feed `: keep-alive\n\n` then reasoning then content then usage then `[DONE]` — confirm event sequence and keep-alive throttle (a stub that emits N events per N bytes fails the throttle check) | today: no SSE parser |
| **REQ-DS-023** | CoT bifurcation default; include_reasoning opts in | Integration | default `.response` is content only; reasoning in a separate stored record; opt-in includes reasoning in `.response` | mock test from T-012: assert `.response` does NOT contain the reasoning text in the default case but DOES contain it when `deepseek_include_reasoning:true`; assert the per-request log record holds the reasoning text in both cases (a stub that always strips fails the opt-in case) | n/a |

## Resilience, timeouts, errors

| REQ | Acceptance | Type | Pass condition | Reality test | Baseline |
|---|---|---|---|---|---|
| **REQ-DS-006** | breaker (3 states), classify by HTTP status, runner owns retry, 4xx hard, 429 honors Retry-After, bypass generic loop | Unit + integration | classify(402)=Hard, classify(429)=Transient, classify(503)=Transient; sequence-based breaker state transitions; agent_exec retries deepseek 0×, codex 3× | as in T-004 + T-013 reality tests; CRITICAL: a "blast-radius" assertion that `execute_ask_agent(agent="codex")` with a failing runner still goes through the existing 3-attempt loop (regression guard) | today: generic retry loop |
| **REQ-DS-007** | Two-timer: 60s rolling read_timeout + 1800s absolute ceiling; tcp_keepalive 30s | Integration | reqwest client config matches; a slow-drip stream completes within ceiling; absolute timeout fires past ceiling | as in T-005 reality test: 90-second slow drip completes (read_timeout keeps resetting); a stalled-after-first-byte stream errors at 60s | n/a |
| **REQ-DS-024** | Fail-loud on either timer; keep-alive emits progress | Integration | timeouts produce explicit errors mentioning "idle" / "ceiling"; no sibling substitution attempted | mock-server test: a stream that pauses 70s after first byte → response is an error containing "idle"; a 2000s stream → error containing "ceiling"; in either case `AskAgentResponse.answered_by_agent` (if set) == "deepseek" (stubs that substitute Gemini fail) | n/a |
| **REQ-DS-029** | Ghost-success: per-chunk error key detection | Unit + integration | embedded `{"error":...}` in a 200 stream classifies like the equivalent HTTP error | mock returns 200 streaming response with `{"error":{"code":"insufficient_balance"}}` mid-stream → runner returns Err classified as HardOpenInsufficientBalance (stub returning Ok fails) | today: HTTP-status-only classification (would silently succeed) |
| **REQ-DS-030** | finish_reason ∈ {length, content_filter, null} → hard error, no partial gibberish, tokens still metered | Unit + integration | runner returns Err on length; usage record present | T-007 reality test + a follow-up: assert that even on length failure, a token record is persisted (a stub that throws away tokens fails) | n/a |
| **REQ-DS-008** | Fail-loud default; no silent sibling substitution | Integration | every error path returns an error to Claude; no path returns Ok with content from another sibling | inject failures across all classes (402/429/timeout/finish_reason/ghost-success/disconnect); ASSERT none of them produce a response whose `.response` matches any other configured sibling's mock output (a stub that ever falls back to Gemini fails) | n/a |

## Token economics & metering

| REQ | Acceptance | Type | Pass condition | Reality test | Baseline |
|---|---|---|---|---|---|
| **REQ-DS-009** | Mapping: output←completion (incl reasoning, no double-add); price rows present | Unit | calculate_cost_usd for v4-pro/flash matches documented numbers | T-003 reality test (within FP tolerance) + T-009 reality test (no double-add) | today: no DS rows |
| **REQ-DS-010** | Prepaid + 402 hard-trip + exact metered + alerts | Integration | 402 → HardOpenInsufficientBalance; balance endpoint readable | (a) a fake 402 trips breaker to Hard (T-013); (b) live probe `GET /user/balance` returns the JSON schema {is_available, balance_infos[…total_balance,granted_balance,topped_up_balance…]} — stubbed handler that returns `{}` fails the schema check | n/a |
| **REQ-DS-021** | Per-consult cost visible (lifecycle/progress line) | Integration | a successful consult emits a cost lifecycle event computed sync from price_table | mock test: a Mock response with known usage → assert an emitted event contains the string `cost:` and a dollar amount that matches calculate_cost_usd for the same inputs ±1¢ (stub emitting `cost: $0.00` always fails) | today: no cost line |
| **REQ-DS-026** | Metering keys on usage-chunk-received (exact ↔ estimated); ALL paid paths meter; Err-path persistence gated to deepseek | Unit + integration | success → exact; disconnect → estimated; codex failure → ZERO records | T-013 reality test (3 cases); critical regression guard: 1000 random failure injections on agent="codex" → 0 token records (stub that persists any Codex Err fails) | today: Ok-only persistence |

## Statelessness, routing, sandbox, anti-bulk

| REQ | Acceptance | Type | Pass condition | Reality test | Baseline |
|---|---|---|---|---|---|
| **REQ-DS-011** | Explicit-route; no auto-router; council deferred | Code-review | source has no auto-routing logic for deepseek; council changes are in skill code, not daemon | grep daemon source for "auto.*route\|route.*deepseek" → 0 hits; assert daemon code does not reference a 4-way council struct | n/a |
| **REQ-DS-012** | No sandbox v1 | Code-review | deepseek path does not invoke sandbox-exec | grep `src/deepseek*` for "sandbox" → 0 hits | n/a |
| **REQ-DS-020** | Stateless: synthetic session_id, no resume, prewarm no-op | Integration | two sequential calls succeed with distinct synthetic ids; no remote-history bytes posted | T-014 reality test | n/a |
| **REQ-DS-025** | Hard byte-size intercept on deepseek path only | Unit + integration | 20KB to deepseek → error; 20KB to gemini → success | T-015 reality test (3 cases incl env override) | today: no intercept |

## Verification & operations

| REQ | Acceptance | Type | Pass condition | Reality test | Baseline |
|---|---|---|---|---|---|
| **REQ-DS-017** | Wave-0 probe battery against live api.deepseek.com; #[ignore] gated | Integration (ignored) | `cargo test -- --ignored deepseek_contract` exits 0 against funded account | T-000 reality test + T-016 (end-to-end probe captures real responses into PROBE_RESULTS.md) | already partially run during ceremony |
| **REQ-DS-018** | Operator runbook | Doc | file exists with all required sections | T-017 reality test (grep ≥6 section headers) | n/a |

## Cross-cutting: post-build acceptance gates
- `cargo test --workspace` PASS (excluding `--ignored`)
- `cargo test -- --ignored deepseek_contract` PASS (requires funded account)
- `cargo clippy --workspace -- -D warnings` ZERO findings
- `cargo fmt --check` no diff
- `grep -r "TRIUMVIRATE_DEEPSEEK_API_KEY=sk-" daemon/` ZERO matches (no leaked keys in source)

## Notes on stubs and reality tests
The discipline (Pythia v2 lesson): **every reality test must fail against a stub that
satisfies the type system.** Patterns used here that pass that bar:
- For mapping tests: assert *specific numeric values* (e.g. cost ±1¢, token counts), not just
  that a field is `Some(_)`.
- For state machines (the breaker): assert *transitions across sequences*, not single calls.
- For mocked HTTP: assert the *exact body bytes sent*, not just that a call was made.
- For regression guards (blast-radius): assert that **other agents are unchanged** under failure.
