# Measurement: cutting Grok's per-turn context from 66K to 14K

**Date:** 2026-08-30 · **Binary:** `grok 1.0.13` · **Auth:** SuperGrok subscription, no API key
**Method:** four runs, identical prompt `reply with the single word pong`, `--max-turns 3`, `num_turns` 1 each.
**Fixtures:** `crates/agent-adapter/tests/fixtures/grok-streaming-{20260830,lean-20260830,isolated-20260830}.jsonl`

---

## Results

| # | Configuration | input tokens | cost | tools | commands | answered |
|---|---|---|---|---|---|---|
| A | As the guide would spawn it | 66,559 | $0.02271 | 420 | 509 | pong |
| B | `--tools "read_file,grep,list_dir"` | 57,474 | $0.01959 | 399 | 506 | pong |
| C | project `.grok/config.toml`, all 9 servers `enabled = false` | 61,315 | $0.02137 | 420 | 509 | pong |
| D | **`HOME=<isolated> GROK_HOME=~/.grok`** | **14,386** | **$0.00493** | **26** | **23** | pong |

**D is a 78% reduction, and 4.6x cheaper per call.** Extrapolated over 1,000 consults: **$22.71 versus $4.93.**

## What did not work, and why

**B, `--tools` allowlist: 14%.** It filters **built-in** tools only. The roughly 394 MCP tools are not built-ins, so
they survive the allowlist untouched. Tool count barely moved, 420 to 399.

**C, project-scoped config: 8%, which is likely cache variance rather than a real effect.** Tool and command counts
were **identical to baseline** (420 / 509), so the servers still loaded. `[mcp_servers.<name>] enabled = false`
appears to govern servers *defined* in config, not ones *inherited* from `~/.claude.json`. Disabling by name does not
reach the inherited set.

## What worked

**D.** The inheritance reads `$HOME/.claude.json`, so moving `HOME` removes the import, while `GROK_HOME` keeps the
real profile for credentials.

```bash
HOME=<daemon-owned dir> GROK_HOME="$HOME_REAL/.grok" grok --no-auto-update --no-alt-screen \
  --output-format streaming-json --cwd "$WORKSPACE" -s "$UUID" -p "$PROMPT"
```

Verified side-effect free:

- **Auth reused, not re-minted.** `~/.grok/auth.json` mtime unchanged at 16:01:30 across the run.
- **Nothing written into the isolated HOME.** The directory was still empty afterward.
- **No MCP servers started.** No new files in `~/.grok/logs/mcp/`.
- **Functionally identical.** Same answer, `exit=0`, zero stderr.

## Grok CAN lazy-load. The inheritance simply bypasses it

The lean 26-tool set still contains **`search_tool`** and **`use_tool`**, which are deferred-tool primitives: search
for a tool by intent, then load its schema on demand. That is the same mechanism Claude Code uses for deferred MCP
tools.

So the 66K is not a limitation of Grok's design. **Grok has the machinery to load tools lazily, and the
`~/.claude.json` import loads all of them eagerly anyway**, shipping 420 schemas in the system prompt on every turn
whether or not any will be used. The capability and the default are in conflict.

Worth watching upstream: if the import is later routed through `search_tool`, D's advantage shrinks and the isolation
workaround can be dropped.

## The remaining 14,386 is the real floor

D is not free. 14K input for a six-word prompt is the base system prompt plus 26 native tool schemas. That is the
adapter's actual per-consult floor, and it is the number to budget against, not zero.

## Recommendation for the adapter

1. **Spawn Grok with an isolated `HOME` and an explicit `GROK_HOME`.** Use a stable daemon-owned directory, not a
   temp path, because the isolated HOME is where Grok would place any future state.
2. **Make it the default for consults, and make it overridable.** An operator who deliberately wants Grok holding the
   full MCP fleet should be able to opt in per call. Suggested: `TRIUMVIRATE_GROK_INHERIT_MCP=1`.
3. **Record `total_cost_usd` per call.** Grok reports it in every `end` event, and at 14K to 66K per consult on a
   fixed monthly subscription, quota burn is the metric that matters and it is invisible without this.
4. **Revisit the guide's `--max-turns 20` default.** Each turn re-ships the context. Twenty turns at the 66K floor is
   a different order of spend than at 14K.

## Not established

- Whether an isolated HOME degrades anything Grok does over a longer, tool-using session. Only a trivial prompt was
  tested, and it invoked no tools.
- Whether `grok agent stdio` inherits the same way. If it does, this finding applies to the ACP transport too.
- Whether the 9 inherited servers can be suppressed selectively rather than all-or-nothing.
