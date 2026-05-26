# TECH_STACK — DeepSeek 4th-sibling integration (v1)

## Runtime
- **Language:** Rust (edition **2024**, workspace-pinned in `daemon/Cargo.toml`).
- **Async runtime:** `tokio` (existing workspace dep; multi-threaded).
- **MSRV / toolchain:** existing daemon toolchain (see top-level `rust-toolchain.toml`).

## HTTP client
- **`reqwest`** — already a workspace dependency. v1 uses it directly (hand-rolled), NOT
  `async-openai`. Rationale: this first-ever API integration needs low-level control of
  `read_timeout`, `tcp_keepalive`, SSE byte-stream parsing, usage-chunk extraction, and HTTP-
  status-based error classification — an SDK abstracts away exactly what we must observe.
- **SSE parsing:** hand-rolled byte/line parser over the streaming response body (~50-100
  lines). Pattern: collect bytes, split on `\n\n` event boundaries, dispatch on `data:` /
  `:` (comment) prefixes. Acceptable alternative: `eventsource-stream` crate IF its
  byte-handling semantics let us reset the rolling read_timeout — verify before adopting.

## Concurrency / backpressure
- **`tokio::sync::Semaphore`** — per-instance permit-based concurrency cap on outbound
  DeepSeek requests; default 8 (daemon-side; DeepSeek's own account cap is 500 v4-pro / 2500
  v4-flash, but a single operator never needs that much).
- **Token bucket** — small hand-rolled bucket on top of the semaphore for soft RPM (default 60).

## Serialization
- **`serde` / `serde_json`** — existing workspace deps; used for the request body, the
  streamed JSON chunks, and the (final) usage chunk.

## Existing daemon crates touched (no new third-party deps)
| Crate | Touched | Why |
|-------|---------|-----|
| `mcp-bridge` | YES | new `deepseek_resilience.rs`, new `deepseek/runner.rs` (or `deepseek.rs`), `is_supported_agent_name` extension, env helpers |
| `triumvirate` | YES | `agent_exec.rs` deepseek arm + bypass-retry + persist-before-Err (DeepSeek-gated); `main.rs` `/status` + session-spawn + (optional) ask_agent tool desc note |
| `agent-adapter` | NO new code | reuses `ParsedAgentResult` and `TokenUsage`; the runner returns these |
| `shared-types` | YES (small) | `AskAgentRequest` gains 4 optional `deepseek_*` fields (REQ-DS-027) |
| `mcp-tools` | YES (small) | `inter_agent` display name + supported-agents fallback list |
| `token-economics` | YES | NEW price rows for `deepseek-v4-pro` and `deepseek-v4-flash` in `price_table` (no schema change — `cached_per_mtok` + cached pricing already exist) |
| `agent-worker` | possibly | `require_reused_worker` semantics for stateless DeepSeek (synthetic session_id, no resume) |

## Configuration (env knobs, REQ-DS-015)
| Env | Default | Purpose |
|-----|---------|---------|
| `TRIUMVIRATE_DEEPSEEK_BASE_URL` | `https://api.deepseek.com/v1` | endpoint (config-swappable for future self-host) |
| `TRIUMVIRATE_DEEPSEEK_API_KEY` | (none — required) | Bearer auth; NEVER logged, NEVER in argv |
| `TRIUMVIRATE_DEEPSEEK_MODEL` | `deepseek-v4-pro` | Pro↔Flash one-line tuning swap |
| `TRIUMVIRATE_DEEPSEEK_MAX_TOKENS` | `32768` | generous shared reasoning+answer budget |
| `TRIUMVIRATE_DEEPSEEK_THINKING` | `enabled` | default thinking on |
| `TRIUMVIRATE_DEEPSEEK_REASONING_EFFORT` | `high` | low/medium→high; xhigh→max |
| `TRIUMVIRATE_DEEPSEEK_READ_TIMEOUT_SECS` | `60` | rolling idle-timeout (primary dead-stream detector) |
| `TRIUMVIRATE_DEEPSEEK_TIMEOUT_SECS` | `1800` | absolute outer ceiling |
| `TRIUMVIRATE_DEEPSEEK_TCP_KEEPALIVE_SECS` | `30` | kernel keep-alive |
| `TRIUMVIRATE_DEEPSEEK_MAX_CONCURRENT` | `8` | daemon-side semaphore cap |
| `TRIUMVIRATE_DEEPSEEK_MAX_RPM` | `60` | daemon-side soft RPM |
| `TRIUMVIRATE_DEEPSEEK_REASONING_CAP_TOKENS` | `0` (disabled) | optional runaway-thinking early-abort; when set, MUST be `< _MAX_TOKENS` |
| `TRIUMVIRATE_DEEPSEEK_LOG_DIR` | `$HOME/.triumvirate/deepseek-logs/` | per-request JSON log (reasoning_content + system_fingerprint + cost) |
| `TRIUMVIRATE_DEEPSEEK_LOG_REASONING_CAP_BYTES` | `262144` (256KB) | size cap on `reasoning_content` written to per-request log |
| `TRIUMVIRATE_DEEPSEEK_BULK_BYTES` | `16384` | anti-bulk byte intercept threshold on `ask_agent` payload |

## Pinned external service contract
- DeepSeek API at `api.deepseek.com/v1`, OpenAI-compatible (Bearer + `Content-Type:
  application/json`), models `deepseek-v4-pro` / `deepseek-v4-flash`; promo→permanent
  2026-05-31 15:59 UTC; provenance in `findings/research-verification.md`.

## Costs in scope (operational, not a tool)
- Per-token metering (v4-pro in-miss $0.435/M, in-hit $0.003625/M, out $0.87/M; v4-flash
  $0.14/$0.0028/$0.28). Surfaced per-consult; ledgered exact when usage chunk arrived.
- Account is prepaid; $0 balance ⇒ 402 on every call (operationally critical — runbook).
