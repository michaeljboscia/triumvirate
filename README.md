# triumvirate

**Three AI agents. One coordination layer. No servers.**

Claude Code, Gemini CLI, and Codex work together — sharing context, delegating tasks, and logging their own work — without you having to be the relay.

Built in 27 days. Partially designed by the agents themselves.

---

## What this is

Triumvirate is a coordination layer for three CLI-based AI agents:

| Agent | CLI | Strength |
|-------|-----|----------|
| [Claude Code](https://claude.ai/code) | `claude` | Orchestration, reasoning, long context |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `gemini` | 2M token context, web search, deep research |
| [Codex](https://github.com/openai/codex) | `codex` | Code generation, refactoring, code review |

The core: an MCP server that gives each agent the ability to **spawn**, **query**, and **dismiss** the others as persistent daemon sessions — with full conversation history, shared scratchpads, and automatic session logging.

---

## How it works

```
Star topology (default)          Triangle topology (Codex→Gemini)

      Claude                           Claude
     /     \                          /
  Gemini   Codex               Codex ──── Gemini
                               (Codex manages Gemini internally)
```

**Star:** Claude orchestrates. You ask Claude something, Claude decides whether to delegate to Gemini (large context, research) or Codex (code review, generation). Claude synthesizes the results.

**Triangle:** Claude dispatches Codex with a large task. Codex decides on its own to spin up a Gemini daemon to load a 4000-line file into a 2M-token context window, ask targeted questions, then dismiss Gemini when done. Claude just gets back a finished review — without ever loading the file itself.

Token economics: **pass paths, not content.** All three agents read files directly from disk. Context windows stay clean.

---

## What's included

### Core: inter-agent MCP server

The `mcp-server/` directory contains a TypeScript MCP server that wraps the Gemini and Codex CLIs with a clean daemon API:

```typescript
// Spawn a persistent session
spawn_daemon(cwd: "/path/to/project") → daemon_id

// Ask questions — full conversation history maintained
ask_daemon(daemon_id, "Read /full/path/to/file.ts and explain the auth flow")

// Dismiss — triggers automatic session log
dismiss_daemon(daemon_id)
```

Each `ask_daemon` is a fresh process (~2-3s for Gemini, ~7s for Codex) with full conversation continuity. No PTY, no sentinels, no TUI.

### Pattern: Gemini quick search

Use Gemini's native MCP tools for real-time web search without leaving Claude's context.

### Pattern: Gemini Deep Research

Dispatch multi-source research tasks to Gemini and poll for results. Gemini does the heavy lifting; Claude synthesizes.

### Pattern: Codex code review

Dispatch Codex for git-aware code review on uncommitted changes, branches, or specific commits.

### Pattern: Codex→Gemini delegated review

For large codebases: Claude dispatches Codex, Codex loads the full codebase into Gemini's 2M context window, asks targeted questions, and returns a complete review — without Claude ever touching the files.

### Example: Batch Claude Deep Research

Automate batch-submission of research topics to claude.ai's Deep Research feature via browser automation. Write a list of topics, walk away, come back to completed research.

See `examples/batch-deep-research/`.

### Session Log Spec

A cross-agent session log standard (`SESSION_LOG_SPEC.md`) so every agent documents its work in a compatible format. Built into `dismiss_daemon` — no manual logging required.

---

## Quick start

### Prerequisites

- Claude Code (`claude`) — [install](https://claude.ai/code)
- Gemini CLI (`gemini`) — [install](https://github.com/google-gemini/gemini-cli)
- Codex (`codex`) — [install](https://github.com/openai/codex)
- Node.js 18+

### 1. Build the MCP server

```bash
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/mcp-server
npm install
npm run build
```

### 2. Wire into Claude Code

Add to `~/.claude.json` under `mcpServers`:

```json
"inter-agent-gemini": {
  "command": "/bin/bash",
  "args": ["/path/to/triumvirate/mcp-server/start-gemini.sh"],
  "env": {
    "GEMINI_CLI_PATH": "/usr/local/bin/gemini",
    "DEFAULT_TIMEOUT_MS": "300000",
    "MAX_TIMEOUT_MS": "600000"
  }
},
"inter-agent-codex": {
  "command": "node",
  "args": ["/path/to/triumvirate/mcp-server/dist/codex/server.js"],
  "env": {
    "CODEX_CLI_PATH": "/usr/local/bin/codex",
    "DEFAULT_TIMEOUT_MS": "300000",
    "MAX_TIMEOUT_MS": "600000"
  }
}
```

### 3. Wire Codex→Gemini (optional)

Add to `~/.codex/config.toml` to give Codex the ability to spawn Gemini:

```toml
[mcp_servers.inter-agent-gemini]
command = "/bin/bash"
args = ["/path/to/triumvirate/mcp-server/start-gemini.sh"]

[mcp_servers.inter-agent-gemini.env]
GEMINI_CLI_PATH = "/usr/local/bin/gemini"
DEFAULT_TIMEOUT_MS = "300000"
MAX_TIMEOUT_MS = "600000"
```

Full setup guides: `docs/setup/`

---

## Session logs

Every daemon writes a structured session log when dismissed — automatically, without you asking.

```
project/
└── session-logs/
    ├── owner--client_domain_repo_feature_20260228_v1_gemini.md
    ├── owner--client_domain_repo_feature_20260228_v1_codex.md
    └── owner--client_domain_repo_feature_20260228_v80_claude.md
```

Three agents, three logs, one shared `session-logs/` directory. Any agent can read any other agent's log to pick up context. See `SESSION_LOG_SPEC.md` for the full standard.

---

## The story

We built this in 27 days — and the agents helped design it.

The inter-agent MCP server went through seven rounds of peer review by Gemini and Codex before we shipped it. They found a git commit catch-22 in the session log design. They caught a daemon cross-talk bug where two concurrent Gemini daemons bled into each other's conversation history. They flagged a hardcoded `_claude.md` filter that would have broken multi-agent log versioning.

The system that lets AI agents coordinate was itself coordinated by AI agents.

That felt worth sharing.

---

## Why not Agent Relay?

[Agent Relay](https://github.com/AgentWorkforce/relay) is a proper pub-sub messaging framework — channels, callbacks, real-time routing. It's the right architecture if you're building agents from scratch.

Triumvirate is different:

| | Triumvirate | Relay |
|--|--|--|
| Infrastructure | Zero — stdio MCP processes | Relay server required |
| Sessions | Stateful — full conversation history | Stateless pub-sub |
| Agents | Wrap existing CLIs | Build on the framework |
| Topology | Orchestrator + optional peer-to-peer | Equal peers |
| Setup | Wire existing tools you have | Build new agents |

If you already have Claude Code, Gemini CLI, and Codex — Triumvirate works with what you have.

---

## Hooks (advanced)

Want session logs that survive context compaction — where Gemini automatically summarizes your Claude transcript before memory is lost and restores it on the next session? That's `Tier 2`: a hooks system built on Claude Code's lifecycle hooks, Gemini's pre-compact hook, and Codex's session hooks.

It works. It took the most iteration to get right. Full reference implementation in `docs/hooks/`.

Fair warning: it requires all three CLIs, their config repos wired up, and tolerance for a week of debugging hook interactions.

---

## License

Apache 2.0

---

## Contributing

Issues and PRs welcome. The agents will review your code — that's not a joke, it's the workflow.
