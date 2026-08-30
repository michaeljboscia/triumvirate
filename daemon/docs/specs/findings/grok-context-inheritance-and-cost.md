<!-- EMDASH_OK: section 4.1 quotes agent_exec.rs:336 verbatim and that source line contains an
     em dash written by its author. The argument turns on what the comment actually says. -->

# Grok inherits your entire Claude Code config, and it costs 66K tokens a call

**Date:** 2026-08-30 · **Binary:** `grok 1.0.13` · **Auth:** SuperGrok subscription via `~/.grok/auth.json`, no API key
**Fixtures:** `crates/agent-adapter/tests/fixtures/grok-streaming-20260830.jsonl` (full) and
`grok-streaming-lean-20260830.jsonl` (tools allowlisted)
**Status:** measured, not inferred. Two real calls.

---

## 1. Headless works on subscription auth alone (guide assumption CONFIRMED)

The guide asserts from documentation that headless works with cached login **or** `XAI_API_KEY`. Verified by running
with `env -u XAI_API_KEY`:

```
exit=0  stdout_lines=37  stderr_bytes=0
```

`XAI_API_KEY` was never set. Auth came from `~/.grok/auth.json` (mode 0600), which is exactly the path the guide's §9
doctor probe checks. **REQ-GROK-017 holds, and the subscription path works headless.**

## 2. Guide sections 3.3 and 3.4 are ACCURATE

Every field the guide predicts is present. Event types observed: `available_commands`, `thought`, `text`, `usage`,
`end`. `end` carries `stopReason, sessionId, requestId, usage, num_turns, modelUsage, total_cost_usd`, plus one the
guide does not list, `total_cost_usd_ticks`.

**REQ-GROK-007 CONFIRMED.** The uuid passed via `-s` came back unchanged as `end.sessionId`:

```
passed  : CD94C2BD-530A-48E7-8EA4-91D7853CE6B0
returned: CD94C2BD-530A-48E7-8EA4-91D7853CE6B0
```

Token fields match section 3.4 exactly: `input_tokens, output_tokens, cache_read_input_tokens,
cache_creation_input_tokens, reasoning_tokens`, and `total_tokens` on `end.usage`.

## 3. THE HEADLINE: 66,559 input tokens to answer "pong"

| Call | input | output | cost | tools advertised |
|---|---|---|---|---|
| A, as the guide would spawn it | **66,559** | 39 | $0.02271 | **420** |
| B, `--tools "read_file,grep,list_dir"` | 57,474 | 31 | $0.01959 | 399 |

The prompt was `reply with the single word pong`. `num_turns` was 1.

### Where it comes from

Grok's own README, lines 2313 / 2335 / 2494:

> **MCP Servers** from `config.toml`, plugins, `~/.claude.json`, and `.mcp.json`
> `~/.claude.json` | MCP servers (Claude Code compat)
> Claude Code plugins can provide skills, commands, agents, hooks, MCP servers, and LSP servers.
> **All component types are discovered and used by Grok at runtime.**

**Grok read `~/.claude.json` and started nine MCP servers**, confirmed by the log directory it created:

```
~/.grok/logs/mcp/{apollo,chrome-devtools,gdrive,gemini,github,playwright,pythia,runpod,triumvirate}.stderr.log
```

Note `grok mcp list` reports **"No MCP servers configured."** The inheritance is invisible to Grok's own tooling.

The fixture shows the escalation in three `available_commands` events: **26 native tools, then 420** once the MCP
servers finish connecting, alongside **509 slash commands** harvested from the operator's Claude skills and plugins
(`council`, `crystallize`, `file-taxonomy`, `api-spend-accountability` are all operator-authored).

### `--tools` is NOT the fix

It filters built-ins only: 420 to 399 advertised, a 14% token saving. The roughly 394 MCP tools are unaffected because
they are not built-ins. **Any mitigation has to stop the `~/.claude.json` inheritance, not filter the toolset.**

The lever that exists: `[mcp_servers.<name>] enabled = false`, and project-scoped `.grok/config.toml` supports
`[mcp_servers]` (and only that section). So a daemon-owned cwd carrying a `.grok/config.toml` that disables the
inherited servers by name is the available mechanism. **Fragile**, because the list must track whatever is in
`~/.claude.json`, which drifts. Untested.

## 4. Consequences for the adapter

### 4.1 The telemetry comment is now wrong

`agent_exec.rs:336`:

```rust
// The model only matters for pricing, and DeepSeek is the only metered sibling — codex and
// gemini run on subscriptions, where one more call costs exactly $0.
```

Marginal dollars, true. **Quota, false.** Grok runs on a $30/month SuperGrok subscription and burns roughly 57K to 66K
input tokens per consult regardless of question size. That is a finite budget consumed invisibly, and this comment is
the reasoning that would justify not tracking it.

`CallTelemetry` is agent-generic (`CallTelemetry::new(&req.agent, ...)`), so PostHog capture arrives free with Slice
D. What does **not** arrive free is cost, because `model` is documented as *"Only meaningful for metered agents
(DeepSeek)"*.

### 4.2 The token-economics deferral is wrong

Guide section 3.4 says of cost: *"not on `TokenUsage` today, put cost in log/`detail` or skip in v1"*, and section 10
defers the token scanner to v2. But **Grok reports its own cost in every `end` event**:

```json
"total_cost_usd": 0.02271336,
"modelUsage": {"grok-4.6-build": {"inputTokens":66559,"outputTokens":39,
                "cacheReadInputTokens":512,"modelCalls":1,"costUSD":0.02271336}}
```

Grok is the first sibling that is **subscription-billed AND self-reporting cost**. Capturing that is cheaper than
skipping it, and skipping it is what makes quota burn invisible. **Move cost capture into v1.**

### 4.3 Access is already granted, by default, and was never decided

The operator's intent was to give Grok the same access as the other peers. It already has more:
gdrive, github, apollo, runpod, pythia, chrome-devtools, playwright, gemini, **and triumvirate**.

`triumvirate__ask_agent` in Grok's toolset means **Grok can drive Gemini, Codex, and DeepSeek**, and once the adapter
lands, recursively itself. That is a real capability that arrived through Claude Code compat rather than a decision.
Note also that the README says **hooks** are inherited too, so operator hooks may run under Grok.

This should be settled deliberately before the adapter dispatches unattended.

## 5. Recommended spec changes

1. **Section 6 env matrix:** add cached-login as a first-class auth path, not a fallback to `XAI_API_KEY`.
2. **Section 9 doctor:** invert the probe order for subscription operators, and report **which** auth is in use, since
   an API key silently bills a different account than the subscription.
3. **Section 3.4:** capture `total_cost_usd` and `modelUsage[].costUSD`. Do not skip.
4. **Section 3.7 forbidden flags:** add `--permission-mode`, `--json-schema`, `--tools`, `--disallowed-tools`.
5. **New requirement:** the daemon must control Grok's MCP and context inheritance explicitly, or accept roughly 60K
   input tokens per consult as the floor and say so in the README.
6. **REQ-GROK-013 (single attempt) gains a second justification.** The guide argued it from turn length. The real
   argument is that each retry re-ships the whole inherited context.

## 6. Still not verified

- Whether disabling inherited servers via project-scoped `.grok/config.toml` actually works, and what the floor
  becomes. **This is the single most valuable follow-up measurement.**
- Exit-code behavior (section 3.6). Only exit 0 has been observed.
- `tool_call` and `tool_call_update` shapes. The trivial prompt invoked no tools, so those rows remain unverified.
- Whether `grok agent stdio` inherits the same context. If it does, the transport question is orthogonal to cost.
