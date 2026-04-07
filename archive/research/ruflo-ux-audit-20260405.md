# Ruflo (Claude Flow) - User Experience Audit
**Date:** 2026-04-05 | **Version audited:** v3.5.51

## 1. User-Facing Commands

Install: `curl -fsSL .../install.sh | bash` or `npx ruflo@latest init --wizard`

Key CLI commands (26 commands, 140+ subcommands):
- `ruflo init` / `ruflo init --wizard` / `ruflo init --codex` / `ruflo init --dual`
- `ruflo agent spawn -t coder --name my-coder` / `ruflo agent list`
- `ruflo hive-mind spawn "Build API" --queen-type strategic --consensus byzantine`
- `ruflo hive-mind status` / `ruflo hive-mind metrics` / `ruflo hive-mind memory`
- `ruflo mcp start` (registers 313 MCP tools in Claude Code)
- `ruflo hooks intelligence --status` / `ruflo hooks teammate-idle --auto-assign true`
- `ruflo worker dispatch --trigger audit --context "./src"` / `ruflo worker status`
- `ruflo embeddings init` / `ruflo embeddings search -q "authentication patterns"`
- `ruflo ruvector init` / `ruflo ruvector benchmark` / `ruflo ruvector optimize`
- `ruflo plugins install -n @claude-flow/plugin-gastown-bridge`

After init, the README promises you "just use Claude Code normally" and hooks handle routing.

## 2. README Promises vs Reality

**Promise:** "Deploy 100+ specialized agents in coordinated swarms." After `init`, "just use Claude Code normally -- the hooks system automatically routes tasks to the right agents."

**Reality (per independent audit, issue #1514):** ~290 of 300+ MCP tools are stubs. Only ~10 tools actually compute (memory/HNSW, embeddings, terminal, sessions). Agents spawn into `{ status: "idle" }` forever. Neural training ignores data and returns hardcoded labels. WASM agents echo input back. The "30-50% token savings" metrics are fabricated (hardcoded baselines, `+=100` per cache hit).

## 3. UX Patterns -- How Users See Progress

- **Hooks inject `[INTELLIGENCE]` patterns** into system reminders on every message -- but Claude ignores this passive text (issue #1497). There is no mechanism to force tool invocation from hook output.
- **Agent status** is available via `ruflo agent list` and `hive-mind status` but agents never leave "idle" state because the orchestration→LLM wire is missing.
- **No real progress visibility.** Users cannot observe agents working because agents do not work. The status commands return static JSON.

## 4. User-Reported Pain Points

| Issue | Problem |
|-------|---------|
| #1530, #1531 | **Hooks add 18-21s latency per prompt.** PageRank on 150MB JSON (graph-state.json alone is 94MB) runs synchronously on every session start/end. Never completes -- must Ctrl+C. |
| #1514 | **"99% Theater" audit.** 290/300+ tools are non-functional stubs. Agents, neural, WASM, workflow, consensus, coordination -- all fake. |
| #1497 | **"Just use Claude Code normally" doesn't work.** MCP tools are deferred; Claude never calls them unless explicitly told. Users must manually configure CLAUDE.md. |
| #1504 | **106 agent definitions = 300K tokens of context bloat.** Most reference MCP servers that don't ship. 15+ agents always fail. 7 duplicate files. |
| #1526 | Auto-memory hook silently drops all session data. |
| #1518 | Intelligence graph-state.json grows to 194MB from duplicate entries. |
| #1524 | Memory database doesn't initialize -- memory_store fails. |

## 5. The Actual Human Experience

A user sitting at a terminal will:
1. Run `npx ruflo@latest init --wizard` (works, ~20s)
2. Start Claude Code in that directory
3. Experience 18-21s latency on every prompt due to hook overhead
4. Notice `[INTELLIGENCE]` noise in system reminders that Claude ignores
5. Try to spawn agents -- they register but never execute anything
6. Discover 300+ MCP tools in context that waste tokens but do nothing
7. Either manually disable hooks in settings.json or abandon the tool

**What actually works:** Vector memory (HNSW search), embeddings, terminal execution, session save/restore. Roughly 10 tools out of 300+.

**Bottom line:** The CLI is extensive and the vision is ambitious, but the gap between README promises and functional reality is enormous. The tool currently adds latency and token bloat while delivering ~3% of advertised capabilities.
