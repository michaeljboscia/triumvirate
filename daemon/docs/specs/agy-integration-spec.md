# Spec: Fold Antigravity CLI (`agy`) into Triumvirate

**Status:** Draft for `/goatrodeo` · **Created:** 2026-05-23 · **Owner:** Mike
**Drop-dead date:** 2026-06-18 (Gemini CLI stops serving Pro/Ultra/free-individual requests).
**Research basis:** `/Users/michaelboscia/projects/triumvirate/research/antigravity/` (v1 synthesis + Claude brief + Gemini brief + live verification + Codex/Gemini daemon scoping, 2026-05-23).

---

## 1. Problem

Triumvirate drives its Gemini sibling by spawning the `gemini` CLI as a subprocess and parsing `--output-format stream-json` (NDJSON) for streamed text, tool events, token stats, and a `session_id` it reuses for multi-turn resume. Source of truth: `daemon/crates/triumvirate/src/agent_exec.rs:1211` (`run_gemini_cli_process_with_session`), `:1332` (batch fallback), and `daemon/crates/agent-adapter/src/gemini.rs` (`GeminiStreamParser`).

On **2026-06-18** the `gemini` binary stops serving. Its replacement, `agy` (Antigravity CLI, Go, TUI-first), is **not** a drop-in: no `stream-json`, no captured conversation id for resume, and stdout is silently dropped when stdout is not a TTY. We accept reduced functionality; we must preserve the **most** functionality possible and have **zero silent failures** at cutover.

## 2. Constraints (hard)

- **C1 — Subscription/OAuth auth only. Never an API key.** (`ANTIGRAVITY_API_KEY` is unsupported in v1.0.x — Issue #78 open — and is prohibited by policy regardless.)
- **C2 — Host is macOS (darwin).** Auth persists via macOS Keychain.
- **C3 — The public agent name stays `gemini`.** No new external agent surface (`mcp-bridge/src/lib.rs:34` supports `gemini`+`codex` only). Backend swap is internal.
- **C4 — Codex path is untouched.** This change affects only the Gemini backend.
- **C5 — Rollback is config-only.** Reverting to `gemini` CLI requires no code change while the binary still serves (until 2026-06-18).

## 3. Constitution alignment (Triumvirate)

1. Claude is the front door — unchanged; the user still talks to Claude.
2. Lifecycle always visible — the agy path MUST emit the same working-state lifecycle events the gemini path emits.
3. Plain language in, structured results out — unchanged at the user surface.
4. Failure is loud, immediate, actionable — an agy auth/exec failure MUST surface a user-visible error naming the cause and the fix (e.g. re-auth), never an empty success.

## 4. Out of scope

- Restoring live token-by-token streaming to the UI (impossible without `stream-json`).
- Routing third-party models (Claude/GPT-OSS) through `agy`/Vertex.
- Migrating Codex.
- Building a new ACP adapter from scratch (the wrapper option is *evaluated*, see REQ-070, not built here).

---

## 5. User stories (with human's-path traces)

### US-1 — One-shot question to the Gemini sibling (the MVP)
User asks Claude to consult the Gemini sibling; gets a plain-text answer.
```
User types (to Claude): "ask gemini how SQLite WAL checkpointing works"
  1s : Claude: "→ Gemini (agy): sent ✓"
  3s : Claude: "→ Gemini (agy): working… (3s)"
 ~8s : Claude: "→ Gemini (agy): responded ✓"
 ~8s : Claude displays the plain-text answer
FAIL : "→ Gemini (agy): FAILED ✗ — auth/exec error: <reason>. Run `agy` once to re-auth."
DEGRADED (REQ-053): "→ Gemini (agy) unavailable — falling back…" → "→ Codex: responded ✓";
       answer prefixed "⚠ Gemini unavailable — answered by Codex"
Hops : Claude session → MCP tool (ask_agent/ask_daemon) → daemon HTTP → agent_exec
       → backend selector → agy via PTY → captured plain text (ANSI-stripped)
       → ParsedAgentResult → MCP response → Claude → user display
```

### US-2 — Cutover safety on the drop-dead date
On/after 2026-06-18, the `gemini` binary fails. The user must already be on `agy`, and any residual gemini-backend call must fail loudly with a remediation message, never silently return empty.
```
US-2: Gemini dispatch on/after 2026-06-18
  Already migrated (backend=agy): identical to US-1.
  Residual gemini-cli backend still selected:
    User types (to Claude): "ask gemini <x>"
    1s : Claude: "→ Gemini (gemini-cli): sent ✓"
   ~Ns : Claude: "→ Gemini (gemini-cli): FAILED ✗ — backend retired
          (gemini CLI stopped serving 2026-06-18). Set
          TRIUMVIRATE_GEMINI_BACKEND=agy and restart the daemon."
  NEVER: an empty body reported as a successful answer.
  Hops : same as US-1; the failure is raised at the agent_exec result,
         carried back through MCP response → Claude → user display.
```

### US-3 — Operator switches backend / rolls back
The operator selects the backend with one environment variable and restarts; no code change.
```
Operator sets: TRIUMVIRATE_GEMINI_BACKEND=agy  (or =gemini-cli to roll back)
  → restart daemon → next Gemini dispatch uses the selected backend
  → log line states which backend served the request
```

### US-4 — Multi-turn continuity (recover phase, may be waived)
A follow-up to the Gemini sibling continues the prior thread without cross-contaminating other concurrent workers. If unachievable reliably, the system degrades to single-turn and tells the user the thread did not carry.
```
US-4: Follow-up to the Gemini sibling (Phase B; degrades if unverified)
  User types (to Claude): "ask gemini a follow-up to that"
  Isolation verified (REQ-043 active):
    1s : "→ Gemini (agy): continuing thread ✓"
   ~8s : answer that reflects the prior turn
  Isolation NOT verified / single-turn floor:
    1s : "→ Gemini (agy): new thread — multi-turn not available ⚠"
   ~8s : a fresh answer; the user is told the prior context did not carry
  NEVER: silently resume another worker's conversation (no host-global -c).
  Hops : same as US-1, plus per-worker isolated state-dir selection before spawn.
```

---

## 6. Requirements

### Backend selection
- **REQ-001** A backend selector chooses between `gemini-cli` and `agy` for the `gemini` agent, read from env var `TRIUMVIRATE_GEMINI_BACKEND` with values exactly `gemini-cli` (default) or `agy`.
- **REQ-002** When `TRIUMVIRATE_GEMINI_BACKEND` is unset or any value other than `agy`, behavior is byte-for-byte the current `gemini` CLI path.
- **REQ-003** The selector lives at the existing execution seam `run_agent_process_with_session` (`agent_exec.rs:1682`); the existing `run_gemini_cli_process_with_session` and `run_gemini_batch_process_with_session` remain present and unmodified for rollback AND because the degraded route (REQ-053) reuses the gemini-cli path while it still serves. Fleet has a second, independent Gemini invocation site (`fleet/src/orchestrator.rs:81`) that must apply the same selection — see REQ-090.
- **REQ-004** `TRIUMVIRATE_GEMINI_STREAMING` is NOT reused to select the backend; it continues to toggle only gemini-cli stream-vs-batch behavior.
- **REQ-005** Each Gemini dispatch logs one line naming the backend that served it (`gemini-cli` or `agy`).

### AGY invocation
- **REQ-010** The agy binary path resolves from `TRIUMVIRATE_AGY_BIN` (default `agy`); extra args from `TRIUMVIRATE_AGY_ARGS`, mirroring the existing gemini connector resolution in `mcp-bridge/src/lib.rs`.
- **REQ-011** The agy command passes the prompt as `-p <message>` and sets stdin to null. (No stdin piping; matches current behavior at `agent_exec.rs:1234-1243`.)
- **REQ-012** The agy command does NOT pass `-o/--output-format`, `-r/--resume`, `--session-id`, or `-c/--continue`.
- **REQ-013** The agy command does NOT pass `--model`; the `--model` faildown/retry schedule (`agent_exec.rs:335`, `:918`) is disabled for the agy backend, which runs as a single attempt with no model override.
- **REQ-014** The agy backend uses a dedicated timeout `agy_connector_timeout()` (env `TRIUMVIRATE_AGY_CONNECTOR_TIMEOUT_SECS`, default **900s**), distinct from the gemini/codex `connector_timeout()` (default 180s, `agent_exec.rs:997`) which is too short for agy's blocking, non-streaming, multi-tool runs. The same value is passed to agy's `--print-timeout`. *(Auto-resolved R1: twin-confirmed; env-overridable.)* **VERIFIED 2026-05-24:** the timeout MUST hard-kill via `SIGKILL` on the process group — agy is a Go binary that ignores `SIGALRM`/soft signals (a `perl alarm` guard failed to kill a hung agy that ran hours at 0% CPU). Use `kill_process_group` + SIGKILL, not a soft signal or `.kill()` alone. **VERIFIED 2026-05-24:** intermittent hangs are real and recurring — 2 observed (one cold-start; one with `ANTIGRAVITY_CONVERSATION_ID` set, suggesting resume/state ops as a trigger) — so the hard-kill + one retry (REQ-020) is load-bearing, not theoretical. **CHARACTERIZED 2026-05-24:** 0/30 hung in a controlled run (incl. the `CONVERSATION_ID`-resume condition) — the hang is a rare non-deterministic transient (~2 in ~52 calls, both early), NOT tied to resume. SIGKILL timeout + one retry is sufficient.
- **REQ-015** If a tool-executing path is required, `--dangerously-skip-permissions` is passed only on that path; the read-only consult path (US-1) does not pass it.
- **REQ-016** ✅ **RE-DECIDED 2026-05-24 — wrap agy in our own `sandbox-exec` profile (option A).** Rationale: `agy --sandbox` is not a filesystem boundary (verified — it wrote to `/tmp`) and `agy -p` auto-executes tools (Issue #45), so Triumvirate spawns agy under a Triumvirate-controlled macOS `sandbox-exec` (`.sb`) profile. **The profile constrains WRITES, never READS** — operator requirement: a workflow that stages on-disk files/artifacts for the sibling MUST let it read them; an over-tight seatbelt causes failed tool calls and wasted quota. Profile shape: `(allow default)` → `(deny file-write*)` → re-allow `file-write*` for an **allowlist** = {workspace cwd, agy state `~/.gemini` + `~/.antigravitycli`, the OS temp dir, `/dev/null` & std streams, plus any per-workflow output dirs explicitly declared}. Reads + network stay default-allow so the agent reaches repo files, staged artifacts (wherever they live), and the Google API. The writable allowlist is **parameterized per dispatch** (workspace + extra staged/output paths injected at spawn). `--dangerously-skip-permissions` stays forbidden on the consult path. ✅ Verified by REQ-062b (below); reusable profile at `research/antigravity/agy-verification/agy-sandbox.sb.template`.
- **REQ-058** (R2) Before spawning agy, if the assembled prompt exceeds a safe threshold `TRIUMVIRATE_AGY_MAX_PROMPT_BYTES` (**VERIFIED 2026-05-24:** macOS ARG_MAX is ~1MB/1048576, not 256KB — a 280KB prompt ran fine — so default raised to **900_000**, under the ~1MB argv+env budget), the dispatch fails loud with "prompt too large for agy (no stdin/file input)" instead of hitting an opaque OS `E2BIG`. agy has no `@file`/`--prompt-file`/stdin path (twin-confirmed), so there is no in-band workaround; oversized consults must be chunked by the caller.

### Output capture (the non-TTY stdout problem)
- **REQ-020** ✅ **REVISED 2026-05-24 (verification):** the non-TTY stdout-drop did NOT reproduce on agy 1.0.1 (7/7 clean piped runs), so the **default capture is plain `Stdio::piped()`** (like the current gemini path) with a **SIGKILL process-group hard-kill timeout + one retry** on hang/empty (covers the single observed cold-start hang; see REQ-014). A **PTY capture path** (`portable-pty`, ANSI-strip, plus the non-interactivity guard below) is retained behind `TRIUMVIRATE_AGY_CAPTURE=pipe|pty` (default `pipe`) for instant mitigation if a future agy version regresses to the drop bug — regression is caught by the health probe (REQ-056), the empty-output canary (REQ-024), and version pinning (REQ-059). **Non-interactivity guard (PTY path only):** sets a non-interactive environment (`CI=1`, `NO_COLOR=1`, `TERM=dumb`, `PAGER=cat`, `GIT_PAGER=cat`), never writes to the PTY master, scans for interactive-prompt patterns (`[y/N]`, `press enter`, `continue?`, `Allow`, `Authorize`, `login`, `Select`) and kills+classifies on match. The pipe path needs only the SIGKILL timeout + retry.
- **REQ-021** `portable-pty` is declared as a dependency in `daemon/crates/triumvirate/Cargo.toml` (it is present in `Cargo.lock` but not yet a declared dep).
- **REQ-022** PTY bytes are read on a dedicated OS thread (`std::thread::spawn`), forwarded to async via a channel; the PTY slave handle is dropped in the parent after spawn so EOF is delivered.
- **REQ-023** Captured bytes are ANSI/CSI-escape-stripped before becoming `response_text`.
- **REQ-024** Success/failure keys off the **exit code** (R1 decision). A non-zero exit is a failure (surface status + captured output). Under the PTY, exit 0 with empty output is a legitimate empty answer, NOT a failure. The empty-output-as-failure heuristic is retained ONLY as a PTY-drop canary while REQ-064 is unverified, and ONLY against the known-non-empty probe prompt (REQ-062). Once REQ-064 confirms exit-code behavior on the live binary, the canary is removed. This matches the existing exit-status-first model (`agent_exec.rs:1317/1367`).
- **REQ-025** The agy result sets `parser_mode = "agy-pty-plain-text"`, `session_id = None`, empty `events`, empty `tool_calls`, and `token_usage = None`.
- **REQ-026** `GeminiStreamParser` is not used on the agy path; it remains in use for the gemini-cli path.

### Authentication (subscription/OAuth, macOS)
- **REQ-030** The daemon never sets, reads, or requires `ANTIGRAVITY_API_KEY` or `GEMINI_API_KEY` for the agy path.
- **REQ-031** Operator runbook documents the one-time interactive `agy` OAuth login on the host before cutover, and verification that a second `agy -p` reuses the token with no prompt. **VERIFIED 2026-05-24:** agy stores credentials as a **plaintext file `~/.gemini/oauth_creds.json`** (mode 600), NOT the macOS Keychain; 7/7 in-session calls reused it with no prompt. Persistence is file-based (long-run expiry across days still unmeasured).
- **REQ-032** The agy backend runs in a process context that can read the user login Keychain. CONFIRMED (R1): the Triumvirate daemon already installs as a macOS **LaunchAgent** (`daemon-core::render_launch_agent`, `~/Library/LaunchAgents/com.triumvirate.daemon-v2.plist`), which runs in the logged-in user session and can read the login Keychain — so no LaunchDaemon remediation is needed. **SUPERSEDED 2026-05-24:** auth is a plaintext file (`~/.gemini/oauth_creds.json`, REQ-031), not the Keychain — so neither the LaunchDaemon/Keychain nor the keychain-unlock concern applies. Any process running as the user reads the creds file; no special launch context is needed beyond running as the correct user. The auth blocker that drove three rounds of concern is empirically a non-issue.
- **REQ-033** `agy` is installed at a stable path (e.g. a fixed symlink) so binary-path-bound Keychain ACLs do not re-trigger an interactive "Allow" prompt on update.
- **REQ-034** When agy emits an auth/keyring error (no token, locked Keychain, interactive prompt in a non-interactive context), the dispatch fails with a user-visible message naming the cause and the remediation (run `agy` once interactively / unlock Keychain).

### Session / multi-turn
- **REQ-040** In the default agy mode every dispatch is single-turn: `session_id` passed in is ignored and `session_id` returned is `None`. **VERIFIED 2026-05-24:** single-turn confirmed as the right call — `ANTIGRAVITY_CONVERSATION_ID` (a real env var in the binary) did NOT yield isolated resumable multi-turn for `-p` (conv B forgot its codeword; conv A hung). The Issue #7 blocker stands.
- **REQ-041** The worker registry (`agent-worker/src/lib.rs`) does not persist a synthetic `session_id` for agy dispatches; absence of a session id does not error.
- **REQ-042** The agy path never passes `-c`/`--continue`/`--conversation`/`-r`/`--resume` in single-turn mode (prevents host-global conversation cross-contamination — Issue #7).
- **REQ-043** (Recover phase, behind a separate flag) An optional per-worker isolated state directory provides multi-turn continuity scoped to one worker. This requirement is GATED on verification (REQ-061) that agy honors a config-home/workspace isolation mechanism without breaking Keychain OAuth. If verification fails, REQ-043 is waived and US-4 degrades to single-turn with a user-visible notice.
- **REQ-044** (R1 decision: single-turn + warn) When the agy backend serves a named session (`spawn_session`/`ask_session`, `mcp-tools/src/inter_agent.rs:103/155/170`) or a daemon HTTP session (`main.rs:2041/2055`) in single-turn mode, the response carries a user-visible notice that multi-turn memory is not available for the Gemini sibling under agy. The system never silently fakes continuity and never injects prior history into the prompt in Phase A. (Triumvirate's own session history still records turns; the notice prevents the user from assuming agy *remembers* them.)

### Observability / failure
- **REQ-050** The agy path emits only honest lifecycle states: `TurnStarted`, `TurnCompleted`/`Error`, plus the existing outer elapsed heartbeat (`agent_exec.rs:453/492`) as liveness. It MUST NOT fabricate tool-call events, token deltas, percent-complete, or inferred internal steps (no real progress signal exists without stream-json). Liveness ("working… Ns") is permitted; semantic progress is forbidden. *(Auto-resolved R1: both twins.)*
- **REQ-051** Capacity/quota detection on the agy path matches captured output against the quota string set defined in REQ-053 (`RESOURCE_EXHAUSTED`/`quota`/`429`/`rate limit`/`capacity`), confirmed/extended by REQ-064 on the live binary. The gemini stderr 429 fast-fail (`agent_exec.rs:1259`) does not transfer because the PTY merges stdout/stderr. A matched capacity signal fails the dispatch with a user-visible message and routes straight to codex per REQ-053 (skipping gemini-cli, shared quota pool). **(verify 2026-05-24):** primary detection should parse the per-dispatch `--log-file` (REQ-100, verified to carry model + error context, ~10KB/call) rather than only the merged stdout/stderr; the exact quota string remains the one open item (REQ-064), captured on first real event.
- **REQ-052** A non-zero agy exit status fails the dispatch with a user-visible message including the exit status.
- **REQ-053** (R1 decision: degraded routing) Env `TRIUMVIRATE_GEMINI_DEGRADED_ROUTE` controls behavior when the agy backend hard-fails (auth/exec/quota — not model content). Chain, in order: (1) retry via the legacy `gemini-cli` backend for as long as it still delivers output — detected by a successful exec, NOT a date check, so it self-disables when gemini-cli stops serving on 2026-06-18; then (2) fall back to `codex` (existing supported path, `agent_exec.rs:934`); then (3) fail loud. Default value `gemini-cli,codex` (the R1 choice); set to `fail` to disable all fallback. Each degraded hop emits a lifecycle line naming the actual backend used; the response surface keeps `agent = "gemini"`. The fallback triggers only for `agent == "gemini"` with backend `agy`. **Failure classification (R2):** quota-class failures (exit≠0 with `RESOURCE_EXHAUSTED`/`quota`/`429`/`rate limit`/`capacity`) SKIP gemini-cli — it shares the same Google subscription quota pool (twin-confirmed), so it would also be blocked — and go straight to codex. Auth/exec/PTY-capture/agy-protocol failures try gemini-cli first. Ambiguous errors are NOT treated as quota. Classification uses a typed internal error enum downgraded to user text at the boundary (`agent_exec.rs:582/1658`). **Substitution honesty (R3):** when a degraded hop answers with a different agent, `AskAgentResponse` carries explicit `answered_by_agent`, `answered_by_backend`, `degraded_from_backend`, and `degradation_reason` fields (today's surface — `shared-types/src/lib.rs:39` — has only `agent`/`response`/`lifecycle`, too thin), AND the response text is prefixed with a one-line notice (e.g. "⚠ Gemini unavailable — answered by Codex") for clients that render only `.response` (e.g. `ask_session`, `inter_agent.rs:155`). The public `agent` field stays `gemini` (what was requested).
- **REQ-054** (R2) The degraded route has a total wall-clock budget `TRIUMVIRATE_GEMINI_DEGRADED_TOTAL_TIMEOUT_SECS` (default 900s). A deadline is set at dispatch start; each hop's timeout is `min(backend_timeout, remaining)`. When the budget is exhausted the dispatch fails loud ("degraded route budget exhausted") rather than stacking agy 900s + gemini-cli 180s + codex sequentially. Implemented in the `execute_ask_agent` attempt loop (`agent_exec.rs:384`).
- **REQ-055** (R2) A global concurrency cap `TRIUMVIRATE_AGY_MAX_CONCURRENT` (default 1) limits simultaneous agy PTY children across BOTH the inter-agent ask path and fleet (REQ-090), via a shared semaphore in the backend module (not duplicated in fleet). When the cap is reached the agy hop queues; the daemon never fans out unbounded PTY children. Rationale: agy shares one subscription quota pool, so unbounded fan-out wastes quota. **VERIFIED 2026-05-24:** 3 concurrent `agy -p` ran clean (all correct, parallel ~7.6s, no shared-state collision) — concurrency is functionally SAFE for single-turn, so the cap is a **quota/resource knob, not a correctness requirement**; default raised to **3** (still bounded). (Capture is pipe by default now, REQ-020.)
- **REQ-056** (R2) A daemon health probe periodically (every `TRIUMVIRATE_AGY_HEALTH_PROBE_SECS`, default 300s) runs `agy -p "2+2"` through the SAME PTY capture path used in production and asserts non-empty output containing `4`. Empty + exit 0 sets `agy_capture_health = degraded` (the PTY-drop-regression signal that production traffic cannot detect — REQ-024); non-zero exit sets `agy_backend_health = failed` with the classified error. Results surface via the existing daemon health/status tools (`main.rs:533/641`), NOT the request path, so legitimate empty answers never fail while a silent PTY-drop regression is still caught.
- **REQ-057** (R2 decision: explicit unmetered) agy dispatches are accounted as **unmetered**, not zero. A schema migration adds `usage_source` (`exact|estimated|unmetered`) to token records; agy rows are written `usage_source=unmetered`, `total_tokens=0`, `cost_usd=NULL`, and are EXCLUDED from cost sums in attribution/queries (`token-economics/src/attribution.rs:47`, `queries.rs:89/134`) and from the cost columns of `get_token_summary`/`get_build_cost`, while still counting as a dispatch occurrence. Dashboards show an honest "unmetered" marker rather than fake-zero spend. This replaces the silent `total_tokens=0` under-report path at `agent_exec.rs:153`. **VERIFIED 2026-05-24:** there is NO headless path to real token counts — `/context`/`/usage`/`/status` are TUI-only (via `-p` they're sent to the model as plain text), asking the model returns a fabricated number (it answered "15"), and no `usageMetadata` appears in stdout or `--log-file`. Real counts live only in the TUI status bar and the raw cloudcode-pa response (neither headless-reachable). So `unmetered` stands; the ONLY alternative for a rough number is a local char-based **estimate** explicitly tagged `usage_source=estimated` (never presented as exact).
- **REQ-059** (R3) agy version is pinned and drift-guarded. `TRIUMVIRATE_AGY_EXPECTED_VERSION` records the verified-good version; the backend checks `agy --version` matches; on mismatch it warns by default, or refuses to start the agy backend if `TRIUMVIRATE_AGY_STRICT_VERSION=true`. **CORRECTED 2026-05-24:** `ANTIGRAVITY_SKIP_UPDATE_CHECK` is NOT a real agy env var (absent from the binary's strings) — do not rely on it. The real update-suppression mechanism is unknown; mitigate a stalled launch-time check via the SIGKILL timeout (kill + retry) and by pinning the binary path. Find a real suppress flag during build, if one exists. Any agy upgrade requires re-running the verification battery (REQ-060–064) and updating the captured artifacts. The `triumvirate doctor` command (`cli_ops::run_doctor`, `cli_ops.rs:59`) is extended to check: agy binary present + executable, version matches expected, OAuth works non-interactively, and the PTY probe `agy -p "2+2"` returns non-empty containing `4`. CAVEAT: Google pushes server-side "harness" updates that can change agy's reasoning/output even with the binary pinned — version pinning bounds local drift only.

### Resilience — model exhaustion / 429 (added 2026-05-24 after prod-429 incident review)
Context: a prior incident saw 429s across most Gemini models. The old gemini-cli mitigations — `--model` faildown (`agent_exec.rs:335/918`) and the stderr 429 fast-fail (`:1259`) — do NOT transfer to agy (no `--model` flag, no structured stream). Resilience therefore moves to the **provider level**.
- **REQ-100 (per-dispatch `--log-file` parsing — the observability substitute).** Every agy dispatch passes `--log-file <per-dispatch temp>` (a LOCAL glog text file — **zero token cost**, ~10KB/call, parse-then-delete). **VERIFIED 2026-05-24 from real log content, recoverable per dispatch:** (a) **serving model**, precisely — `model_config_manager.go: Propagating selected model override to backend: label="Gemini 3.1 Pro (High)"`; (b) **auth method / subscription-vs-Vertex** — `applyAuthResult: authMethod=consumer, quotaProject=` (empty quotaProject ⇒ flat subscription, not metered Vertex); (c) backend endpoint + error/quota lines. **NOT recoverable:** token usage (no usage counts in the log ⇒ REQ-057 `unmetered` stands). **Parsing caveat:** startup emits transient `"not logged into Antigravity"` / `Failed to get OAuth token` warnings BEFORE auth completes even on a SUCCESSFUL call — parse FINAL state / specific patterns (`Propagating selected model`, a real quota line), NEVER a generic error-grep (it false-positives on every call). This replaces the lost stream-json `init.model` and error events.
- **REQ-101 (provider circuit breaker).** A stateful breaker on the agy/Gemini backend: on repeated quota/429 within a window, OPEN the circuit and route ALL Gemini-sibling work to **codex** (different provider/quota pool — NOT gemini-cli, which shares the exhausted Gemini pool, verified). Half-open probe after an exponential-backoff cooldown (align to the ~5-hr Ultra window when a quota reset is the cause). This prevents the retry-storm that turns a transient 429 into an outage.
- **REQ-102 (backpressure / rate limit).** A token-bucket max-RPM limiter plus the concurrency cap (REQ-055) throttle Triumvirate's own agy call rate to avoid self-inflicted 429s (a likely aggravator of the prior incident). Configurable; conservative default.
- **REQ-103 (retry policy — anti-storm).** Classify each failure (REQ-051 + exit code + `--log-file`): **transient/network/hang** → retry once with exponential backoff + jitter; **quota/429** → NO immediate retry (it would just 429 again) → trip the breaker (REQ-101) → codex; **permanent/bad-input** → fail loud, no retry. Until the exact quota signal is verified (REQ-064), bias AMBIGUOUS repeated failures toward the breaker (treat as quota) — the prior incident shows storms are the worse failure mode.
- **REQ-104 (observability stance).** Google's official observability path for agy is NOT the CLI — the gemini-cli `--telemetry`/OTLP flags were removed (confirmed: absent from `agy --help`), and there is no structured JSON metrics output. Google points headless users to the **Antigravity SDK** (Python) or the Managed Agents API. **VERIFIED 2026-05-24:** the official `google-antigravity` PyPI SDK (author Google LLC, Apache-2.0, repo `Google-Antigravity/antigravity-sdk-python`, I/O-day release) authenticates via **`GEMINI_API_KEY` only** — its quickstart/docs show no consumer-subscription/OAuth path and it depends on `google-genai` (API-key/Vertex). **The SDK is therefore NOT usable under C1 (subscription-only, no API keys).** The ONLY subscription-programmatic path is calling the internal `cloudcode-pa.googleapis.com/v1internal` endpoint directly with the agy OAuth token (as the CLI does) — but that is an undocumented private API requiring IDE-identity-spoofing headers (`Client-Metadata`), i.e. fragile + grey-area ToS → **rejected escape hatch, not a plan.** Conclusion: **wrapping the agy CLI is the officially-sanctioned way to use the subscription programmatically.** Observability stays DIY: `--log-file` parsing (REQ-100) + Triumvirate's existing telemetry (working-state events, ledger, latency, exit codes, routing/breaker state); token usage stays `unmetered` (REQ-057) — it cannot be recovered under subscription-only without the grey-area path.

### Verification gates (BLOCKING — must pass on a live `agy` install before build)
- **REQ-060** ✅ **VERIFIED 2026-05-24 (v1.0.1):** `agy --help`/`--version` captured in `research/antigravity/agy-verification/help.txt`. NO `--output-format` (REQ-012/026 stand), NO `--model`/`-m`, `--sandbox` exists, `--print-timeout` default 5m. The captured flag list overrides any flag assumption in this spec.
- **REQ-061** ✅ **VERIFIED 2026-05-24:** real home is `~/.gemini/`; transcripts at `~/.gemini/config/projects/<uuid>.json` (symlinked from `~/.antigravitycli/`); `~/.antigravity/` does not exist (the research's MCP path was wrong). See FINDINGS.md.
- **REQ-062** The non-TTY stdout-drop is reproduced (`agy -p "2+2" | cat` empty vs. PTY non-empty), confirming REQ-020 is required (vs. waivable if stdout works under a pipe). ✅ **DONE 2026-05-24:** `agy --sandbox` did NOT block an out-of-workspace write (agy wrote to `/tmp`); `--sandbox` is not a filesystem boundary. Escalation required per REQ-016 (reopened). Network confinement not separately tested — the write failure already invalidates `--sandbox` as a boundary.
- **REQ-062b** ✅ **VERIFIED 2026-05-24:** the Triumvirate-controlled `sandbox-exec` profile (REQ-016) was prototyped and proven (`probe4`): an out-of-allowlist write was BLOCKED even when agy attempted it via built-in file tool, shell, and python ("Operation not permitted"); a staged artifact OUTSIDE the workspace was READ successfully; network worked; agy ran normally and wrote within the workspace. Reusable profile: `research/antigravity/agy-verification/agy-sandbox.sb.template`.
- **REQ-063** ✅ **VERIFIED 2026-05-24 (in-session):** one-time OAuth then repeated `agy -p` reuse the token with no prompt (7/7). Creds file shows access token ~1 hr (auto-refreshed) and a refresh token with **no local expiry stamp** → durable absent Google-side revocation. Multi-day re-auth cadence still needs instrumentation (auth-failure logging) to fully confirm.
- **REQ-064** ◐ **PARTIALLY VERIFIED 2026-05-24:** success = 0, bad flag = 2 (`flags provided but not defined` on stderr). Quota/RESOURCE_EXHAUSTED exit code still unsampled (would require deliberately exhausting the 5-hour window) — confirm opportunistically.

### Wrapper option (evaluate, do not assume)
- **REQ-070** (R2 decision: KEPT as a gated Phase-B evaluation — zero Phase-A scope; both twins + research confirmed no OAuth streaming path exists by 2026-06-18, so this is a post-MVP bet, not a dependency) Before committing to the PTY+plain-text baseline as permanent, the `atomr-agents-coding-cli-vendor-antigravity` crate (`rustakka/atomr-agents`) and the ACP-adapter approach are evaluated against the live `agy` binary for: (a) subscription-OAuth-only operation, (b) restoring streaming/structured output, (c) maintenance/production-readiness. If one demonstrably works under C1, it supersedes REQ-020–REQ-026 in a later phase. If not, the PTY baseline stands and this requirement is closed as "evaluated, rejected."

### Fleet — second backend site (R1 decision: in scope)
- **REQ-090** Fleet's Gemini path (`fleet/src/orchestrator.rs:81`, which today spawns `gemini -p` with `Stdio::piped()`, bypassing `agent_exec`) honors `TRIUMVIRATE_GEMINI_BACKEND`; when `agy`, it captures via PTY (REQ-020) and does NOT use the `Stdio::piped()` path (which would hit the silent-drop bug). Prefer routing fleet's Gemini execution through the shared backend selector rather than duplicating the spawn logic.
- **REQ-091** Fleet Gemini dispatches are single-turn under agy (consistent with REQ-040) and pass no resume/continue flags.
- **REQ-092** Fleet Gemini failures honor the degraded route (REQ-053) or fail loud; a fleet task never reports success on empty agy output (REQ-024).

### Testing
- **REQ-080** A mock `agy` binary backs subprocess tests: it emits nothing on exit 0 when stdout is not a TTY (`[ -t 1 ]` false) and emits ANSI-colored text when stdout is a TTY.
- **REQ-081 (reality test)** The agy capture test asserts the response equals the stripped plain text from the mock — a path using `Command::output()`/`Stdio::piped()` captures empty and FAILS; only a real PTY implementation passes. (A stub cannot satisfy this.)
- **REQ-082** A concurrency test asserts two simultaneous agy dispatches pass no `-c/--continue/--conversation/-r/--resume` and persist no synthetic session id.
- **REQ-083** A rollback test asserts that with `TRIUMVIRATE_GEMINI_BACKEND=gemini-cli` the gemini-cli path still receives `-o stream-json` and parses the NDJSON fixture.

---

## 7. Phasing

- **Phase A — MVP, before 2026-06-18 (preserve single-turn dispatch):** REQ-001–005, 010–015, 020–026, 030–034, 040–042, 050–052, 060–064, 080–083. Outcome: single-turn plain-text Gemini-sibling dispatch over agy, subscription-OAuth, loud failures, config-only rollback.
- **Phase B — Recover (after MVP is safe):** REQ-043 (multi-turn via isolated state dir, if verified), REQ-070 (wrapper/ACP evaluation to restore streaming/metadata). Each gated on live-binary verification.

---

## 8. Decisions (Round 1 resolved + still open)
1. ~~**Worker context** (LaunchDaemon vs LaunchAgent)~~ — **RESOLVED R1:** it's a LaunchAgent → auth viable (REQ-032).
2. ~~**Single-turn acceptance**~~ — **RESOLVED R1:** accept single-turn for Phase A with a user-visible warning (REQ-040/044).
3. ~~**Failure semantics**~~ — **RESOLVED R1:** exit-code primary, empty-canary until REQ-064 (REQ-024).
4. ~~**Fleet scope**~~ — **RESOLVED R1:** in scope (REQ-090–092).
5. ~~**Degraded mode**~~ — **RESOLVED R1:** gemini-cli (while serving) → codex → fail (REQ-053).
6. ~~**Capacity detection fidelity**~~ — **RESOLVED R2:** classify quota via exit≠0 + `RESOURCE_EXHAUSTED`/`quota`/`429` text (REQ-053); confirm exact strings in REQ-064.
7. ~~**Wrapper bet**~~ — **RESOLVED R2:** kept as a gated Phase-B eval, zero Phase-A scope (REQ-070).
8. ~~**Large-prompt handling**~~ — **RESOLVED R2:** no agy file/stdin path exists; guard with fail-loud over ARG_MAX (REQ-058); caller must chunk.

## Appendix A — Code insertion points (non-normative, from Codex daemon scoping)
- Backend dispatch: `agent_exec.rs:1682` (`run_agent_process_with_session`).
- New fn `run_agy_cli_process_with_session` beside `agent_exec.rs:1211`.
- New `agy_command()` + `gemini_backend()` in `mcp-bridge/src/lib.rs` (~`:220`), reusing `resolve_connector_command`.
- `has_any_arg` (`agent_exec.rs:1155`) matches exact tokens only — will miss `--prompt=value`; generate space-form args or extend the helper.
- Session writeback at `agent_exec.rs:525`; gemini session backfill at `:1326`/`:1379` (skip for agy).
- Prewarm (`agent_exec.rs:976`) prewarms gemini+codex; gate gemini prewarm off for agy single-turn.

## Appendix B — Verification battery (carried from the synthesis; satisfies REQ-060–064)
`agy --help` · `agy --version` · `agy -p "2+2" | cat` (expect empty) vs PTY (expect "4") · `ls -la ~/.gemini ~/.antigravity ~/.antigravitycli` · one-time OAuth then second `agy -p` no-prompt · exit-code sampling.
