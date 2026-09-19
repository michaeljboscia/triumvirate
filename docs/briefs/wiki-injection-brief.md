# Brief: inject the bosciamem wiki map into peer calls, and check whether it was used

> **SUPERSEDED FOR GROK, 2026-09-19 15:47 UTC.** Grok now receives the map from
> `/Users/michaelboscia/.grok/rules/bosciamem-wiki.md`, which grok 1.0.30 loads as a global user rule.
> Verified through this bridge: `ask_agent` grok, request `bd869ac0-af28-4479-bb4b-add17c77a4f1`, answered
> the page count and top page with zero tool calls. The canary below was wrong because it tested files
> Grok does not load under this config. **Do not build injection for Grok: it would put the map in twice.**
> All four seats are now reached by files, so nothing in this brief is needed for delivery. The measurement
> half is still needed; see `wiki-usage-measurement-brief.md`. Evidence:
> `/Users/michaelboscia/projects/mneme-bosciamem/research/2026-09-19-grok-rules-delivery.md`.


Owner's request, 2026-09-19: "doing wiki checks and injections when the CLIs get called via Triumvirate."

The measurement half is `/Users/michaelboscia/projects/triumvirate/docs/briefs/wiki-usage-measurement-brief.md`.
Item 4 below is summarised there in full, with the guards and controls it needs; build against that file, not this paragraph.

## Why the bridge is the right place
Each peer CLI loads instructions differently, verified today with canary tokens rather than from documentation:

| peer | what it actually loads |
|---|---|
| Codex | `~/.codex/AGENTS.md` and a project `AGENTS.md` (project canary seen) |
| Gemini, through the Antigravity CLI | `~/.gemini/GEMINI.md` only, the global one. A project `GEMINI.md` and a project `CLAUDE.md` were both invisible |
| Grok, through Triumvirate | nothing. Its own words: "Injected harness user_rules and system reminders are not file reads." |

So the file route reaches two of three seats and can never reach Grok. The bridge reaches all three, is the only place that knows which agent is being called, and is already the place that composes the prompt.

Codex and Gemini now receive the map through their own files, written by `ops/build_index.py --agents` in the mneme repo into a managed block. That means the bridge must not blindly inject for every agent or those two get it twice, which panel P13 explicitly refused ("Do not simultaneously inject another copy").

## Where the code is
`daemon/crates/triumvirate/src/agent_exec.rs`:
- `inject_tool_marker_prompt(user_prompt) -> String` at line 1553 composes `TOOL_MARKER_INSTRUCTIONS` plus the user request.
- Called at line 488 as `let execution_prompt = inject_tool_marker_prompt(&req.message);`, after the cwd is resolved and before the worker is acquired.

That single call site is the seam. Everything below is a change to it and to config.

## Proposed
1. **Config, per agent, default off.** `wiki.inject = {grok: "map", codex: "none", gemini: "none"}` plus `wiki.map_path`, defaulting to `~/projects/mneme-bosciamem/_index.compact.md`. Values: `none`, `map`. A future `map+pages` value is out of scope here. Per-agent defaults exist because two seats already get the map from their own files; the setting records why.
2. **Injection.** When the value is `map`, prepend the map inside a fenced block that says what it is: a map, not instructions, with the UNREVIEWED warning the wiki index already carries. Place it before the tool marker instructions so the operating rules stay last and closest to the request.
3. **Staleness.** Read the map's generation date from its first line. If it is older than `wiki.max_age_days` (default 7) or the file is missing, inject nothing and record the reason. A silently stale map is worse than none, which is the same lesson as the breaker.
4. **The check.** After the turn, scan the response for page ids from the map and record one event per call: agent, whether a map was injected, its generation date, how many page ids appeared in the response, and the request id. This is item 1 and item 2 of `~/projects/mneme-bosciamem/research/2026-09-19-usage-measurement.md` implemented where it is cheapest, because the bridge already sees both sides of the exchange.
5. **Kill switch.** `TRIUMVIRATE_WIKI_INJECT=0` disables it everywhere without a rebuild, and the per-agent config can turn a single seat off.

## Acceptance
- With `grok: "map"`, a Grok call that asks how many pages the wiki has answers correctly from the prompt alone, with no file reads (today it answers NONE).
- With `codex: "none"`, a Codex call contains exactly one copy of the map, the one from its own `AGENTS.md`.
- A map file dated older than `max_age_days` is not injected, and the skip is recorded with its reason.
- The per-call event records `page_ids_referenced` and is queryable, so the referenced-after-injection rate can be read without parsing transcripts.
- No test asserts on wiki content; assertions are on presence, count and provenance.

## Not in scope
Injecting page bodies keyed to the prompt. That is the retrieval upgrade and needs a relevance measure first, per the consumption research and P13. This brief delivers the map and the measurement that would justify going further.
