# triumvirate

**Three AI agents. One coordination layer. No relay server.**

Claude Code, Gemini CLI, and Codex work together — sharing context, delegating tasks, and documenting their own work — without you having to be the relay.

Built in 27 days. Partially designed by the agents themselves.

---

## What this is

Triumvirate is a coordination layer for three CLI-based AI agents:

| Agent | CLI | Strength |
|-------|-----|----------|
| [Claude Code](https://claude.ai/code) | `claude` | Orchestration, reasoning, long context |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `gemini` | 2M token context, web search, deep research |
| [Codex](https://github.com/openai/codex) | `codex` | Code generation, refactoring, code review |

The core: an MCP server that gives each agent the ability to **spawn**, **query**, and **dismiss** the others as persistent daemon sessions — with full conversation history and shared scratchpads.

---

## How it works

```
Star topology (default)            Triangle topology (Codex→Gemini)

       Claude                              Claude
       │    │                              │
       │    │                              │
  Gemini    Codex                   Codex ──── Gemini
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
// Spawn a named session — survives MCP restarts, resumes with zero token cost
spawn_daemon({ session_name: "arch-auditor", cwd: "/path/to/project" }) → daemon_id

// Ask questions — full conversation history maintained across calls
ask_daemon(daemon_id, "Read /full/path/to/file.ts and explain the auth flow")

// Soft dismiss (default) — session preserved on disk, resumable later
dismiss_daemon(daemon_id)

// Hard dismiss — permanently delete session files
dismiss_daemon(daemon_id, { hard: true })

// Next session: resume instantly with no re-feed
spawn_daemon({ session_name: "arch-auditor" }) → "Gemini daemon resumed (existing session)."
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

### System: Stenographer — zero-cost session notes

Stenographer is an incremental transcript narrator that runs on every Claude session — no API calls, no cloud tokens, just local Ollama.

The token gate hook fires every ~50K tokens. Stenographer reads only the new transcript bytes since the last save (delta extraction), parses the JSONL into normalized events, feeds them to a local Ollama model, and appends a plain-English paragraph to the session log.

```
transcript.jsonl  →  parse delta  →  local Ollama  →  session-log.md
                      (new bytes      (qwen2.5:32b    (appends one
                       only)           or similar)     paragraph)
```

Why local? The pre-compact hook used to pipe full Gemini transcripts to the Gemini API for summarization — that burned 69M tokens in 4 days. Stenographer costs $0.00 per save and produces a rolling narrative the next session can resume from.

Supported transcript formats: Claude Code JSONL, Codex JSONL, Gemini JSON arrays.

See [`starter-kit/stenographer/`](starter-kit/stenographer/) for setup and configuration.

### System: The Airlock — file snapshot safety net

The Airlock (`pre-tool-use-artifact-guard.sh`) snapshots every file before Claude edits it. No confirmation prompts — it just runs silently on every `Edit` and `Write` call and writes a timestamped backup to `~/.claude/artifact-guard/`.

Three protection levels: `remote_strict` (Supabase SQL — checks backup freshness + hash match, blocks if stale), `remote_best_effort` (edge functions, n8n JSON — always snapshots, always allows), `local_copy` (source files — snapshots, always allows).

The result: every edit is reversible, even when Claude makes a mistake at 2am.

### Session Log Spec

A cross-agent session log standard (`SESSION_LOG_SPEC.md`) so every agent can document its work in a compatible format that all three can read and resume from.

### Ecosystem: Batch Claude Deep Research

Automate batch-submission of research topics to claude.ai's Deep Research via browser automation. Separate project, works well alongside Triumvirate.

→ [github.com/michaeljboscia/claude-deep-research](https://github.com/michaeljboscia/claude-deep-research)

---

## Quick start

### Prerequisites

- [Claude Code](https://claude.ai/code) (`claude`)
- [Gemini CLI](https://github.com/google-gemini/gemini-cli) (`gemini`)
- [Codex](https://github.com/openai/codex) (`codex`)
- Node.js 20+, jq, git
- [Ollama](https://ollama.com) — required for Stenographer (local session notes). Pull any general model, e.g. `ollama pull qwen2.5:32b`

### One-command setup

```bash
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/starter-kit
chmod +x install.sh
./install.sh
```

The installer:
1. Copies Claude Code hooks (8 files) to `~/.claude/hooks/`
2. Installs Claude settings, CLAUDE.md starter template
3. Copies Codex hooks, skills, and config to `~/.codex/`
4. Copies Gemini hooks and GEMINI.md to `~/.gemini/`
5. Builds the inter-agent MCP server (`npm install && npm run build`)
6. Wires all three agents' MCP configs (Claude → Gemini+Codex, Gemini → Codex, Codex → Gemini)
7. Installs Stenographer to `~/.triumvirate/stenographer/` (local Ollama session notes)
8. Creates `~/.ai-memory/` (git-initialized session log store)
9. Copies `.env.example` and `taxonomy.json.example` templates

Safe to re-run — backs up existing files before overwriting.

### Post-install

```bash
# Set up credentials (at minimum: GEMINI_API_KEY for pre-compact summarization)
cp ~/.claude/.env.example ~/.claude/.env
# Edit ~/.claude/.env with your API keys

# Create your first project (must be a git repo)
mkdir -p ~/projects/my-project/.claude
cd ~/projects/my-project && git init
cp ~/.claude/taxonomy.json.example .claude/taxonomy.json
# Edit taxonomy.json with your project details
git add .claude/taxonomy.json && git commit -m "init: add taxonomy"

# Start working
claude
```

See [`starter-kit/README.md`](starter-kit/README.md) for full documentation on hooks, session logs, and configuration.

### Manual setup

If you only want the MCP server (no hooks or session persistence), see [`docs/setup/`](docs/setup/) for individual wiring instructions.

---

## Session logs

Triumvirate defines a cross-agent session log standard (`SESSION_LOG_SPEC.md`) — a shared markdown format so every agent documents its work in a way all three can read.

Session logs are AI working memory — they don't belong inside project repos. They go to a dedicated private memory repo:

```
~/.ai-memory/                          # or $AI_MEMORY_DIR
└── my-project/
    ├── owner--client_domain_repo_feature_20260228_v1_gemini.md
    ├── owner--client_domain_repo_feature_20260228_v1_codex.md
    └── owner--client_domain_repo_feature_20260228_v80_claude.md
```

Set up your memory repo:

```bash
mkdir -p ~/.ai-memory && cd ~/.ai-memory && git init
# Optional: push to a private remote
gh repo create ai-memory --private -y
git remote add origin git@github.com:yourname/ai-memory.git
```

Set `AI_MEMORY_DIR` to point to your private memory repo (defaults to `~/.ai-memory`). If the directory doesn't exist, logs fall back to `<project>/session-logs/`.

**Automatic logging:** When you `dismiss_daemon`, the agent writes a `SESSION_LOG_SPEC`-compliant log before the session closes. No manual `/save-session` needed — dismiss produces a log.

Three agents, three logs, one shared directory. Any agent can read any other agent's log to pick up context across sessions.

See `SESSION_LOG_SPEC.md` for the naming convention, required sections, git workflow, and cross-agent compatibility rules.

---

## The story

We built this in 27 days — and the agents helped design it.

The inter-agent MCP server went through seven rounds of peer review by Gemini and Codex before we shipped it. They found a git commit catch-22 in the session log design. They caught a daemon cross-talk bug where two concurrent Gemini daemons bled into each other's conversation history. They flagged a hardcoded `_claude.md` filter that would have broken multi-agent log versioning.

The system that lets AI agents coordinate was itself coordinated by AI agents.

That felt worth sharing.

---

## Security posture

Both Gemini and Codex CLIs apply their own sandboxes by default. Triumvirate disables these for the MCP daemon context:

- **Codex:** `--dangerously-bypass-approvals-and-sandbox` — removes Codex's seatbelt, allowing outbound network and unrestricted file access
- **Gemini:** `--approval-mode yolo` — removes Gemini's approval prompts and file access restrictions

These are the CLIs' own documented escape hatches, intended for controlled programmatic environments. Triumvirate is that environment — the MCP server controls what prompts reach the agents. Only use Triumvirate in trusted contexts on your own machine.

---

## Roadmap

Nothing on the list right now — ship fast, add things when the need is real.

### What shipped

- **Starter Kit** — one-command installer for the complete multi-agent operating environment. See [`starter-kit/`](starter-kit/).
- **Hook lifecycle** — 8 Claude hooks + 3 Gemini hooks + 2 Codex hooks. Session recovery, auto-staging, The Airlock (file snapshots before edit), token-gated auto-save, Gemini-powered transcript summarization before context compaction.
- **Stenographer** — zero-cost incremental session notes via local Ollama. Token gate fires every ~50K tokens; Stenographer reads only new transcript bytes, narrates them locally, appends to the session log. No API calls, no cloud cost. See [`starter-kit/stenographer/`](starter-kit/stenographer/).
- **The Airlock** — silent file snapshot safety net on every edit. Three protection levels: strict (blocks if backup is stale), best-effort (always snapshots, always allows), copy (source files). Every edit reversible, zero prompts.
- **Automatic session logs on dismiss** — every daemon writes a `SESSION_LOG_SPEC`-compliant log when dismissed. Gemini and Codex both produce structured markdown with taxonomy, context summary, and transcript history.
- **`computeAgentLogPath` shared utility** — agent-aware log path computation that reads `.claude/taxonomy.json` for project taxonomy.
- **Daemon persistence** — `spawn_daemon({ session_name: "..." })` creates named sessions that survive MCP restarts. Soft dismiss preserves session files. Resume with zero token cost.
- **Revive-on-ask** — dead daemons probe once on the next `ask_daemon` before giving up permanently.
- **Model fallback chain** — Gemini quota exhaustion falls back automatically through the model chain.
- **Inter-agent skills** — Codex skills for `/send-to-claude`, `/send-to-gemini`, `/send-to-codex`, `/send-to-siblings` (dual escalation).

PRs welcome. The agents review their own contributions.

---

## Hooks

The starter-kit includes a complete hook lifecycle for all three agents. These are installed by `install.sh` and work out of the box.

| Hook | Agent | Event | What it does |
|------|-------|-------|-------------|
| `session-start.sh` | Claude | SessionStart | Project picker (from HOME) or session recovery (from project) |
| `post-compact-recovery.sh` | Claude | SessionStart:compact | Restores context from session log after compaction |
| `pre-compact.sh` | Claude | PreCompact | Gemini summarizes transcript → session log → git commit |
| `post-tool-use.sh` | Claude | PostToolUse | Auto-stages files, logs activity |
| `post-tool-use-token-gate.sh` | Claude | PostToolUse | Auto-saves at ~50K token intervals — triggers **Stenographer** (local Ollama narration) |
| `pre-tool-use-artifact-guard.sh` | Claude | PreToolUse | **The Airlock** — snapshots every file before edit |
| `pre-tool-use-bash-guard.sh` | Claude | PreToolUse | Blocks destructive SQL without fresh backup |
| `session-start.sh` | Gemini | SessionStart | Session log recovery |
| `pre-compact.sh` | Gemini | PreCompact | Self-summarization → session log → git commit |
| `post-tool-use.sh` | Gemini | PostToolUse | Auto-stages files, logs activity |

The hooks are extensible. Wire PM tools (Linear, Jira, GitHub Projects) into the lifecycle using the same pattern: source a shared library, run in a background subshell `( ... ) &`, gate with an env var.

See [`starter-kit/README.md`](starter-kit/README.md) for full hook documentation and configuration variables.

---

## License

Apache 2.0

---

## Contributing

Issues and PRs welcome. The agents will review your code — that's not a joke, it's the workflow.
