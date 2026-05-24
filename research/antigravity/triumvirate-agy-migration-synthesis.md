# Triumvirate → Antigravity CLI (`agy`) Migration: Synthesis & Build Brief

**Purpose:** Hand-off document for Claude Code to begin implementing Triumvirate's migration from `gemini-cli` to `antigravity-cli` (`agy`).
**Method:** Synthesis of two independent deep-research passes — one by Claude (Anthropic), one by Gemini (Google) — on the same target, cross-checked against primary sources where they disagreed.

**Created (v1):** 2026-05-23
**Source research artifacts:** Claude DR (2026-05-23), Gemini DR (2026-05-23)

---

## 0. READ THIS FIRST — Verify before you build

The two research passes **agree** on the core architecture verdict but **contradict each other** on two load-bearing facts. Do **not** write production code against the contested claims until you've run these one-line tests on a real `agy` install. The build plan in §4 branches on the results.

| # | Test command | If it works | If it fails |
|---|---|---|---|
| T1 | `agy -p "what is 2+2" \| cat` | Non-TTY stdout is fine. PTY **not** mandatory. Plain-text capture OK for one-shot. | The "stdout suppression" bug is real/universal. PTY or ACP layer **required**. |
| T2 | `ANTIGRAVITY_API_KEY=xxx agy -p "test"` (fresh machine, no prior OAuth) | Headless auth is trivial via env var. | No API-key auth in this version. Must do interactive OAuth bootstrap on host once; keyring takes over. |
| T3 | `agy --help 2>&1 \| tee agy-help.txt` | Authoritative flag list. **This overrides every claim in this doc.** | — |
| T4 | `ls -la ~/.gemini ~/.antigravity ~/.antigravitycli 2>/dev/null` | Reveals the **real** config/state layout (the two reports disagree — see §3). | — |
| T5 | `echo "hi" \| agy -p` | (expected to fail with `flag needs an argument`) — confirms stdin-piping is dead. | If it works, stdin ingestion survived; simplifies large-prompt handling. |
| T6 | `agy -p "test"; echo "exit=$?"` then repeat with a deliberately bad flag | Maps `agy`'s exit-code taxonomy (undocumented). Triumvirate's control flow branches on this. | — |

**Capture all six outputs and feed them back before Phase 1.** Everything downstream depends on T1 and T2.

---

## 1. Confidence-tiered findings

Legend: ✅ **Verified** (primary source or both reports + corroboration) · 🟡 **Corroborated** (multiple secondary sources, no primary) · 🟠 **Contested** (the two reports disagree) · 🔴 **Unverified** (single source, plausible, unconfirmed)

### Binary & invocation
- ✅ Binary is `agy`, written in Go, TUI-first; shares engine with Antigravity 2.0 desktop. Not a 1:1 drop-in. ([Google Developers Blog](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/))
- ✅ `-p` / `--print` / `--prompt` run a single prompt non-interactively, emit plain text. ([Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7))
- ✅ `-p` takes the prompt **only** as a flag arg; piping the prompt body via stdin fails with `flag needs an argument`. ([gsd-build #3782](https://github.com/gsd-build/get-shit-done/issues/3782)) → confirm with **T5**.
- ✅ `--include-directories` → `--add-dir`. ([Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7), gsd-build)
- ✅ `--yolo` → `--dangerously-skip-permissions`. ([gsd-build #3782](https://github.com/gsd-build/get-shit-done/issues/3782))
- ✅ No headless `-m`/`--model` flag; model picked internally or via in-TUI `/model`. ([gsd-build #3782](https://github.com/gsd-build/get-shit-done/issues/3782))
- 🟡 `--sandbox` flag exists; OS sandbox via nsjail (Linux) / sandbox-exec (macOS) / AppContainer (Windows). *(Gemini DR; sandbox backends unverified against primary.)*
- 🔴 **Loss of `--approval-mode plan` (read-only mode) for non-interactive runs.** Headless `agy -p` reportedly auto-approves file-writing tools unless constrained. **Security-relevant for untrusted input.** *(Gemini DR only — verify before relying on `agy` for any untrusted-input path.)*

### STDIN / STDOUT / IPC
- 🟠 **Stdout suppression in non-TTY contexts.** Gemini DR: universal, numbered "Issue #76", PTY mandatory. Claude DR: intermittent/length-dependent, issue number unconfirmed. **Corroborating evidence the phenomenon is real:** a [community MCP bridge exists specifically to work around a headless `agy -p` stdout bug by reading transcript files](https://github.com/SinanTufekci/Claude-Code-Antigravity-CLI-MCP-Server/blob/main/test_smoke.py), and there's an [independent headless TTY-lock/pipe-deadlock report](https://github.com/devanshug2307/antigravity-discussions/discussions/52). **→ T1 settles it.**
- ✅ Blocking/synchronous: `agy -p` runs to completion (~5s typical, longer for multi-tool) then returns the whole payload. No token-by-token streaming from the bare binary.

### Output schema
- ✅ `--output-format stream-json` (NDJSON: `init`/`message`/`tool_use`/`tool_result`/`result`) is **gone** in v1.0.0. ([gemini-cli baseline schema](https://github.com/google-gemini/gemini-cli/pull/10883); absence confirmed by [gsd-build #3782](https://github.com/gsd-build/get-shit-done/issues/3782))
- 🟠 `--output-format json` (bulk object). One widely-cited [dev.to tutorial](https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7) shows it; hands-on testers say it doesn't exist in v1.0.x. **→ confirm via T3.** If present, the legacy `{response, stats, error}` parser may port with only field renames.

### Conversation / multi-turn (the hard blocker)
- ✅ `agy --print` never surfaces a conversation ID to stdout/stderr/any documented file. ([Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7))
- ✅ `--conversation <id>` resumes a **known** ID but cannot capture one from a `--print` run; `-c`/`--continue` resumes the **most-recent conversation globally on the host** → catastrophic cross-contamination for N concurrent agents. ([Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7))
- 🔴 **Hooks may carry `conversationId` + `workspacePaths` in their stdin JSON payload** — a *potential side-channel* to capture the ID that `--print` won't emit. **If true, this is the cleanest fix to the multi-turn blocker.** *(Gemini DR only — high value, must verify.)*
- 🔴 Transcript files at `~/.antigravitycli/<uuid>.json`; schema undocumented, filer calls it "fragile to rely on." ([Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7))

### Auth & config paths
- 🟡 OAuth creds → OS keyring (Keychain/Credential Manager/libsecret over D-Bus), not plaintext. Sign-out via in-TUI `/logout`. ([dev.to](https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7))
- ✅ Headless Linux / WSL2 keyring persistence is a known failure mode → repeated interactive OAuth prompts that hang a daemon. ([Google AI Forum WSL2 bug](https://discuss.ai.google.dev/t/bug-antigravity-cli-agy-fails-to-persist-authentication-state-in-wsl-2-environment/146059))
- 🔴 **Headless keyring fix (Gemini DR):** install `dbus-x11` + `libsecret-tools`, force-create `~/.local/share/keyrings`, export `DBUS_SESSION_BUS_ADDRESS` before spawning `agy`. Cited error string: `consumerOAuth: failed to persist token to keyring: failed to unlock correct collection`. *(Actionable if accurate — verify.)*
- 🟠 **Config layout.** Claude DR: `~/.antigravitycli/` (transcripts) + `~/.antigravity/mcp_config.json` (MCP). Gemini DR: `~/.gemini/antigravity-cli/settings.json` + `~/.gemini/antigravity/mcp_config.json`. **→ T4 settles it.** Context files (`GEMINI.md`/`AGENTS.md`) unchanged per both.
- 🟠 **`ANTIGRAVITY_API_KEY` env var.** Gemini DR: works, recommended primary headless-auth path. Claude DR: not implemented, only open feature request ([Issue #78](https://github.com/google-antigravity/antigravity-cli/issues/78)); the closest real artifact uses `ANTIGRAVITY_CLI_PATH` (a binary-path override, [agent-bridge README](https://huggingface.co/algorembrant/agent-bridge/blob/main/README.md)), which is easy to conflate. **→ T2 settles it. Do not assume it works.**

### Auth tier / cost (Mike's specific situation — Consumer AI Ultra, `oauth-personal`)
- ✅ Legacy `gemini-cli` stops serving Pro/Ultra/free-individual users **2026-06-18**. Migration mandatory; ~4-week runway. ([Google Developers Blog](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/))
- 🟡 Core Gemini models via `agy` stay on flat-rate Ultra. Third-party models (Claude, GPT-OSS) route through **Vertex AI Model Garden** → per-token GCP billing + `cloud-platform` scope + a configured GCP project. ([Antigravity OAuth scopes](https://github.com/NoeFabris/opencode-antigravity-auth/blob/main/docs/ANTIGRAVITY_API_SPEC.md))
- 🟡 Ultra ≈ 25,000 monthly AI credits ([Google One Help](https://support.google.com/googleone/answer/16286513?hl=en)); Claude-under-Ultra quota [reported as often unusable](https://discuss.ai.google.dev/t/ultra-subscription-claude-model-quota-even-worse-than-pro/135870). **Don't architect heavy Claude routing through `agy` without stress-testing your real quota.**
- ✅ Escape valve: a paid Gemini API key / enterprise license keeps `gemini-cli` alive past June 18 — the only consumer-tier continuity path Google left intact.

---

## 2. What each pass uniquely contributed

**Gemini DR's unique value (fold in):**
1. **The ACP-adapter architecture** — put an [Agent Client Protocol](https://agentclientprotocol.com/get-started/clients) bridge in front of `agy` to restore streaming + a stable JSON-RPC stdio schema. ACP is real (JSON-RPC, Zed+JetBrains origin) and [Antigravity is a confirmed ACP adopter](https://www.beforethecommit.com/episode-26-agent-client-protocol-and-antigravity/). **Caveat:** the specific binary name Gemini cited (`agy-acp`) is **unconfirmed** — find the actual adapter before depending on it.
2. Loss of plan-mode (read-only) security regression.
3. Concrete headless-Linux keyring bootstrap recipe.
4. Hooks possibly exposing `conversationId` (potential multi-turn workaround).
5. `--add-dir`-per-thread isolation as a session-scoping workaround.

**Claude DR's unique value (keep):**
1. The entire **Consumer AI Ultra cost/auth-tier analysis** — Vertex billing for Claude/GPT-OSS, AI credits, broken Claude quota. (Central to Mike's situation; Gemini omitted it.)
2. The **escape hatch**: paid API key keeps `gemini-cli` alive past June 18; **Path C** = bypass the CLI and call the API/SDK directly from Rust.
3. **Exit-code taxonomy** matters for subprocess control flow (→ T6).

**Reliability caveat on Gemini DR:** it cites many `antigravity.google/docs/*` pages (cli-using, cli-features, hooks, mcp, gcli-migration) as if read — those are the exact client-rendered SPA pages that returned **no** server-side content to Claude. Either Gemini rendered the JS or confabulated. Its most *specific* claims (exact config paths, hook schema, `agy plugin import gemini`) are simultaneously its most actionable **and** most at-risk. Treat them as 🔴 until T3/T4 confirm.

---

## 3. Recommended target architecture

Pick the path **after** T1/T2 results land:

- **Path A — ACP bridge (preferred if T1 fails or you need streaming/multi-turn).** Triumvirate speaks ACP JSON-RPC to an adapter that wraps `agy`. Restores streaming, normalizes schema, sidesteps the stdout/TTY bug, and gives per-session isolation. Cost: a new Rust ACP client + dependency on a third-party adapter whose exact identity must be confirmed.
- **Path B — PTY + synchronous bulk parse (fallback if no acceptable ACP adapter exists).** Spawn `agy` inside a pseudo-terminal (`portable-pty` / `expectrl`), strip ANSI, parse the whole payload synchronously. Multi-turn isolated via per-thread `--add-dir` working dirs. No streaming.
- **Path C — Bypass the CLI entirely (cleanest long-term).** Call the Gemini API (paid key) or Antigravity SDK directly from Rust; Triumvirate owns the agent harness. Largest rewrite, but removes all CLI fragility and the June 18 clock pressure for the Gemini path.
- **Path D — Raw `Stdio::piped()` plain-text (only if T1 passes AND multi-turn isn't load-bearing).** Simplest; viable for stateless one-shot workloads (summarize/classify) only.

**Decision rule:** T1 pass + single-turn-only → D. T1 pass + multi-turn → B or C. T1 fail → A or C. Heavy Claude/GPT-OSS routing needed → factor in Vertex/GCP setup regardless of path.

---

## 4. Build plan for Claude Code

**Phase 0 — Verify (blocking).** Run T1–T6 on a real `agy` install. Record outputs. Resolve all 🟠 contested claims and the high-value 🔴 (hooks `conversationId`, plan-mode loss, keyring recipe). Do not proceed until T1 and T2 are answered.

**Phase 1 — Command builder rewrite.**
1. Binary literal `gemini` → `agy`.
2. Delete `--output-format json`/`stream-json` branches (re-add only if T3 shows `json` exists).
3. Delete `--session-id`/`--resume`; add `--conversation <id>` only post-ID-capture (knowing first call can't create one).
4. `--include-directories` → repeated `--add-dir`.
5. `--yolo` → `--dangerously-skip-permissions`; **audit which tools were previously gated** (plan-mode is gone).
6. Set `--print-timeout` explicitly.

**Phase 2 — IPC layer (path-dependent, per §3).** PTY+ANSI-strip, or ACP client, or direct API. Replace the async NDJSON `LinesCodec`/`serde_json::from_str`-per-line parser with whatever the chosen path emits.

**Phase 3 — Multi-turn state.** Implement the conversation-ID strategy: hooks side-channel (if verified) → else per-thread `--add-dir` isolation → else transcript-file capture with a global mutex (race-prone, last resort). Never use bare `-c`/`--continue` from a concurrent context.

**Phase 4 — Auth & config probes.** Move from `~/.gemini/settings.json` + `~/.gemini/oauth_creds.json` to the layout T4 reveals + OS keyring (Rust `keyring` crate). For the daemon host: one-time interactive OAuth bootstrap before June 18; apply the D-Bus/keyring recipe if headless Linux (after verifying it). Decide model-routing policy: Gemini-only (free under Ultra, never invoke `/model`) vs. Claude/GPT-OSS (requires GCP project + Vertex + cost monitoring).

**Phase 5 — Resilience & cutover.** Map exit codes (T6). Instrument for (a) empty-stdout on long prompts and (b) conversation cross-contamination. 48h soak under realistic concurrency. **Flip prod ≤ 2026-06-15** (3-day buffer). Fallback: provision paid Gemini API key, keep `gemini-cli` running.

**Re-decision triggers:** [Issue #7](https://github.com/google-antigravity/antigravity-cli/issues/7) merged → Path B/safe multi-turn becomes easy. `--output-format json` ships → near-drop-in JSON parser. [Issue #78](https://github.com/google-antigravity/antigravity-cli/issues/78) (API-key auth) ships → headless auth trivial.

---

## Revision history
- **v1 — 2026-05-23** — Initial synthesis of Claude DR + Gemini DR, confidence-tiered, with verification battery and Claude Code build plan.

## Bibliography
- Google Developers Blog (Lyalin, Mullen), *An important update: Transitioning Gemini CLI to Antigravity CLI* — https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/
- antigravity-cli Issue #7 (conversation-ID gap) — https://github.com/google-antigravity/antigravity-cli/issues/7
- antigravity-cli Issue #78 (API-key auth feature request) — https://github.com/google-antigravity/antigravity-cli/issues/78
- gsd-build/get-shit-done Issue #3782 (hands-on `agy --help`, flag behavior) — https://github.com/gsd-build/get-shit-done/issues/3782
- SinanTufekci, Claude-Code-Antigravity-CLI-MCP-Server (transcript-file stdout workaround) — https://github.com/SinanTufekci/Claude-Code-Antigravity-CLI-MCP-Server/blob/main/test_smoke.py
- devanshug2307 antigravity-discussions #52 (headless TTY-lock/pipe-deadlock) — https://github.com/devanshug2307/antigravity-discussions/discussions/52
- agent-bridge README (`ANTIGRAVITY_CLI_PATH` usage) — https://huggingface.co/algorembrant/agent-bridge/blob/main/README.md
- Agent Client Protocol — clients/adapters directory — https://agentclientprotocol.com/get-started/clients
- Before The Commit, Ep.26 (Antigravity confirmed ACP adopter) — https://www.beforethecommit.com/episode-26-agent-client-protocol-and-antigravity/
- Arindam Majumder (DEV), *Antigravity CLI hands-on guide* — https://dev.to/arindam_1729/antigravity-cli-a-hands-on-guide-to-googles-terminal-coding-agent-5bc7
- gemini-cli PR #10883 (legacy stream-json schema baseline) — https://github.com/google-gemini/gemini-cli/pull/10883
- opencode-antigravity-auth API spec (OAuth scopes) — https://github.com/NoeFabris/opencode-antigravity-auth/blob/main/docs/ANTIGRAVITY_API_SPEC.md
- Google AI Developers Forum (WSL2 keyring persistence bug) — https://discuss.ai.google.dev/t/bug-antigravity-cli-agy-fails-to-persist-authentication-state-in-wsl-2-environment/146059
- Google AI Developers Forum (Ultra Claude quota worse than Pro) — https://discuss.ai.google.dev/t/ultra-subscription-claude-model-quota-even-worse-than-pro/135870
- Google One Help (AI Ultra 25,000 monthly credits) — https://support.google.com/googleone/answer/16286513?hl=en

---

### Sourcing note on the Gemini DR
The Gemini deep-research document this synthesis incorporates additionally cited several `antigravity.google/docs/*` pages and a handful of Reddit threads (e.g. "Issue #76", `agy plugin import gemini`, `~/.gemini/antigravity-cli/settings.json`, hook schema). Those specific claims could not be independently confirmed against primary sources because the official docs site is a client-rendered SPA that served no content to Claude's fetcher. They are carried here as 🟠/🔴 and gated behind T3/T4 rather than dropped, because several are high-value if accurate. Verify on a live install before building against them.
