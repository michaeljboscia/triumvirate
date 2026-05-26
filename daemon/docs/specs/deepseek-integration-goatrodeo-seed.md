# DeepSeek → Triumvirate — /goatrodeo SEED BRIEF

> **What this is:** a problem brief to feed into a fresh **/goatrodeo** session (Phase 0 →
> interrogation → research → twin review → decision ledger). It is NOT a spec — it is the
> raw material the interrogator should tear into. Goal of the new session: produce a
> goatrodeo-reviewed spec for **adding DeepSeek as a sibling to the Triumvirate.**
>
> **First move in the new session:** read this, then run `/goatrodeo` against it. Consult
> BOTH twins (Gemini + Codex) — they disagree by design, and the access-model decision
> below is exactly the kind of fork where that matters.

---

## 0. The goal (one line)
Add **DeepSeek** to Triumvirate as a usable sibling, so Claude can consult it alongside
Gemini and Codex.

## 1. The strategic thesis goatrodeo should pressure-test FIRST
**DeepSeek is a genuinely different model family — so it should be a FIRST-CLASS agent
(a real 4th voice: `gemini`, `codex`, `deepseek`), NOT hidden behind an existing one.**

This is the *opposite* conclusion from the just-completed agy (Antigravity) work, and the
contrast is the whole point:
- **agy was Gemini re-plumbed** (same model, Gemini 3.x, same quota pool). Exposing it as a
  peer would have been "asking Gemini twice" — no adversarial diversity. So agy was kept as
  a *hidden backend* behind the `gemini` agent (transparent cutover). Gemini-the-twin
  reviewed that decision and confirmed it.
- **DeepSeek is a different vendor/architecture** (DeepSeek V4 / R1 / V3.2, MoE, MIT
  open-weights). It fails differently from Claude (Anthropic) and Codex (OpenAI/GPT). That
  is the entire value of adversarial cooperation — a fourth seat that actually disagrees.

So the default recommendation into goatrodeo is: **expose `deepseek` as its own agent.**
Let the interrogator + twins try to break that, but that's the starting position.

## 2. The central decisions goatrodeo MUST adjudicate (the real forks)

### Fork A — Access model (the big one; this is a genuine "what to build" §0 ambiguity)
DeepSeek has **no consumer subscription** (no ChatGPT-Plus / Gemini-subscription
equivalent). Programmatic access is therefore one of:
1. **DeepSeek API** — `api.deepseek.com`, **OpenAI-compatible**, **API-key** auth, very
   cheap (V4 ≈ $0.30/M in, $0.50/M out; V3.2 cheaper; 5M-token free grant). Fastest to wire.
   **Tension:** the user's standing rule is *"subscriptions only, never API keys"*
   ([[auth-subscriptions-only-never-api-keys]]). **But that rule was framed for Gemini**,
   which *has* a subscription. DeepSeek has none. So goatrodeo must explicitly ask the user:
   **does the no-API-keys rule bind DeepSeek, or was it Gemini-specific?** Do not assume.
2. **Self-hosted open weights** — V3/R1/V4 are MIT-licensed; run locally (ollama / vLLM /
   llama.cpp → OpenAI-compatible local endpoint). **No API key, sovereign, no per-token
   cost** — fits the user's ethos. **Cost:** serious GPU (these are large MoE models;
   quantized R1/V3 still need real VRAM). The user has GPU build plans (`docs/vulcan-1-*`,
   `mx-k3s-gpu`, `mx-gpu-*` skills) → self-hosting is a live option, not hypothetical.
3. **Route through an existing OpenAI-compatible coding CLI** (OpenCode / aider / similar)
   pointed at the DeepSeek endpoint (cloud or local). Reuses Triumvirate's CLI-subprocess
   pattern with minimal new code. DeepSeek is officially supported by such tools.

**This fork drives everything else** (auth, cost, the integration shape, sandbox needs).
Resolve it with the user before specifying.

### Fork B — Integration shape (subprocess vs native API)
Every current Triumvirate sibling is a **CLI subprocess** (`gemini`, `codex`, `agy`).
DeepSeek has **no first-party CLI** — it's API + web + open weights. So:
- **Subprocess path:** wrap a CLI that speaks DeepSeek (OpenCode/aider/thin wrapper, or
  `ollama run`). Fits the existing pattern (mirror the codex backend) → low architectural risk.
- **Native-API path:** Triumvirate makes direct HTTP calls (httpx-style, but in Rust) to the
  DeepSeek OpenAI-compatible endpoint. Cleaner for an API model, but introduces a NEW agent
  pattern (all agents are currently subprocesses). More code, less precedent.
goatrodeo should pick one and justify it.

### Fork C — Which model + what role
V4 (flagship general), **R1 (reasoning, shows its chain-of-thought)**, V3.2 (cheap workhorse).
Is `deepseek` a **coding** agent (compete with codex), a **general** agent (compete with
gemini), or a **reasoning specialist** (R1 for hard logic/math the others punt on)? The role
shapes the model choice + how Claude routes to it.

## 3. Triumvirate architecture context (where a new agent plugs in)
The agy work mapped these seams precisely. **Verify line numbers on a fresh checkout — code
drifts** — but the structure is current as of the agy merge (May 2026, `main`):

- **Supported-agents gate:** `daemon/crates/mcp-bridge/src/lib.rs:~36` (`is_supported_agent_name`
  — currently `gemini || codex`). This single line is where C3-style "what agents exist" is
  enforced. Adding `deepseek` here makes it a first-class agent.
- **Dispatch seam:** `daemon/crates/triumvirate/src/agent_exec.rs` — `run_agent_process_with_session`
  (`~1658`, the `match agent {…}`) and `run_named_agent_with_session_and_model` (`~908`). Add a
  `"deepseek"` arm.
- **Orchestration:** `execute_ask_agent` (`~206`) — attempt schedule, retries, resilience
  gating, degraded route, token persistence. A new agent flows through here.
- **Command/connector resolution:** `mcp-bridge/src/lib.rs` — `resolve_connector_command` (`~267`),
  `gemini_command`/`codex_command` (`~220`). Add `deepseek_command()` + env (`TRIUMVIRATE_DEEPSEEK_BIN`/`_ARGS`).
- **Codex backend = the reference for a 3rd-party agent:** `run_codex_cli_process_with_session`,
  `codex_command`, `codex_protocol` (exec vs app-server), `codex_capabilities` probe. DeepSeek-as-
  subprocess should mirror this.
- **Resilience pattern:** `mcp-bridge/src/agy_resilience.rs` (circuit breaker + token-bucket
  rate limit + concurrency cap + half-open lease + reset-window cooldown). For a metered API,
  rate-limit + breaker still apply (429s, spend caps); generalize or copy.
- **Token economics:** `daemon/crates/token-economics/` — `TokenRecord`/`usage_source`
  (`exact|estimated|unmetered`), `price_table`, `attribution.rs::calculate_cost_usd`,
  `queries.rs`. **DeepSeek differs from agy here:** the API returns **real** input/output token
  counts → record `usage_source=exact` with a **real `price_table` entry** (V4/R1/V3.2 prices).
  This is metered, real-cost spend — unlike agy's `unmetered`.
- **MCP surface:** `ask_agent {agent,message}` already takes an agent param (`mcp-tools/src/inter_agent.rs`,
  daemon HTTP `/ask-agent`). Adding `deepseek` to the supported set exposes it through the
  existing tool — no new tool needed.
- **Misc to touch:** `display_agent_name` (mcp-tools), prewarm (`agent_exec` ~976), the
  daemon `/status` `supported_agents` list (main.rs), the MCP env in `~/.claude.json`.
- **Sandbox:** only if the DeepSeek agent **executes tools/writes files** (a coding CLI would).
  Reuse the agy `sandbox-exec` containment pattern (`mcp-bridge/src/agy.rs`,
  consult = no-workspace-write). A pure Q&A/reasoning agent needs no sandbox.

## 4. Patterns & constraints to carry over from the agy work
- **Resilience is mandatory, not optional** — agy shipped with breaker + rate-limit +
  degraded route + SIGKILL-timeout, and Codex's adversarial review caught real bugs in all of
  them over 3 rounds. DeepSeek needs the same rigor (esp. if API: 429s, spend runaway, timeouts).
- **Degraded route:** what happens when DeepSeek fails? Probably → fail loud or → another
  sibling. Define it.
- **Token/cost accounting:** metered (exact) — add a price entry; surface real spend (agy was
  excluded as unmetered; DeepSeek is NOT).
- **Adversarial review discipline:** the agy lesson was *don't build in a vacuum*. Run both
  twins on the design (Gemini) and the code (Codex) — that's what /goatrodeo formalizes.
- **No timeline words** in the spec (§5). Use dependency/scope language.

## 5. Reference implementation (the template)
The **agy integration is the worked example of "add a backend/sibling."** Read it:
- Spec: `daemon/docs/specs/agy-integration-spec.md` (+ `-HANDOFF.md`, `-BRIEF.md`).
- Code (on `main`): the `feat(agy)` commits — `agy.rs`, `mcp-bridge::agy` / `agy_resilience`,
  the `agent_exec` selector/degraded route, token-economics `usage_source`, fleet second site.
- Operator runbook: `daemon/docs/agy-operator-runbook.md` (env-knob + deploy pattern to mirror).
- Verification discipline: `research/antigravity/agy-verification/` (probe battery — do the
  equivalent for DeepSeek: prove the access path, auth, token counts, failure signals against
  the real thing before building).

## 6. Open questions for the interrogator to drive out
1. **Access model** (Fork A) — API key vs self-host vs CLI-proxy. *Blocking.*
2. **Does "no API keys" bind DeepSeek?** (it has no subscription) — *blocking, user decision.*
3. Coding vs general vs reasoning role (Fork C) — drives model + routing.
4. Native-API vs subprocess (Fork B).
5. If self-hosted: which hardware (vulcan-1?), which serving stack (vLLM/ollama), which
   quantization — and is that a dependency that must land first?
6. Degraded behavior + spend cap (if metered).
7. Does Claude route to DeepSeek automatically (when?) or only on explicit "ask deepseek"?

## 7. Live facts (grounded May 2026 — verify in the new session, DeepSeek moves fast)
- Models: **V4** (flagship, launched ~Mar 2026), **R1** (reasoning, exposes thinking), **V3.2**
  (cheap workhorse). All **MIT open-weights** (self-hostable).
- API: `api.deepseek.com`, **OpenAI-compatible**, **API-key**, pay-per-token (V4 ≈ $0.30/M in /
  $0.50/M out; V3.2 ≈ $0.28/$0.42, cache hits ~$0.028/M in), ~5M-token free grant. **No
  subscription product.**
- DeepSeek is natively supported as a backend by coding CLIs/agents (OpenCode, etc.).
- Sources: DeepSeek API docs (https://api-docs.deepseek.com/), pricing
  (https://api-docs.deepseek.com/quick_start/pricing), BentoML model guide
  (https://www.bentoml.com/blog/the-complete-guide-to-deepseek-models-from-v3-to-r1-and-beyond).

## 8. Relevant standing context (memory / CLAUDE.md)
- [[auth-subscriptions-only-never-api-keys]] — the no-API-keys rule (Gemini-framed; re-examine for DeepSeek).
- [[agy-migration-side-by-side]] — the shadow-compare pattern; could A/B DeepSeek vs others similarly.
- [[gemini-code-quality-vs-codex]] — twins fail differently; DeepSeek adds a 3rd failure mode.
- Consult **both** twins; `dispatch_codex`/`_worktree` is for Codex *writing code in a repo* only,
  not analysis/review (CLAUDE.md §10).
