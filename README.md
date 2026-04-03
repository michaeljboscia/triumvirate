# triumvirate

**Three AI agents. One coordination layer.**

[![License: FSL-1.1-ALv2](https://img.shields.io/badge/license-FSL--1.1--ALv2-blue.svg)](LICENSE)
[![Claude Code](https://img.shields.io/badge/built%20with-Claude%20Code-blueviolet)](https://claude.ai/code)
[![Gemini CLI](https://img.shields.io/badge/uses-Gemini%20CLI-4285F4)](https://github.com/google-gemini/gemini-cli)
[![Codex CLI](https://img.shields.io/badge/uses-Codex%20CLI-74aa9c)](https://github.com/openai/codex)

Claude Code, Gemini CLI, and Codex work together — sharing context, delegating tasks, and documenting their own work — without you having to be the relay.

**Human?** [Start with the Plain English Guide](docs/plain-english-guide.md) — no jargon, no assumptions.
**AI Agent?** [Skip to the Agent Setup Guide.](#agent-setup-guide)

---

## The Story

We built this in 27 days — and the agents helped design it.

The inter-agent MCP server went through seven rounds of peer review by Gemini and Codex before we shipped it. They found a git commit catch-22 in the session log design. They caught a daemon cross-talk bug where two concurrent Gemini daemons bled into each other's conversation history. They flagged a hardcoded filename filter that would have broken multi-agent log versioning.

Everything in this repo exists because someone screwed up first. The Airlock exists because Claude overwrote a production config at 2am with no backup. The Stenographer exists because the old pre-compact summarizer burned 69 million API tokens in four days. The "persist or fail" skill exists because 5 hours of batch results evaporated from memory without ever hitting disk.

The guardrails aren't theoretical. They're scar tissue.

---

## How It Works

```
Star topology (default)            Triangle topology (Codex→Gemini)

       Claude                              Claude
       │    │                              │
       │    │                              │
  Gemini    Codex                   Codex ──── Gemini
```

**Star:** You talk to Claude. Claude delegates to Gemini (2M-token context, web search) or Codex (code review, generation), synthesizes the results.

**Triangle:** Claude dispatches Codex with a large task. Codex autonomously spins up a Gemini daemon, loads a 4000-line file into Gemini's context, asks targeted questions, dismisses Gemini, and returns a complete review. Claude never touches the file.

**Token economics:** Pass paths, not content. All three agents read files from disk. Context windows stay clean.

---

## What's Included

### Inter-Agent MCP Server

The core. A TypeScript MCP server wrapping Gemini and Codex CLIs with a daemon API — `spawn_daemon`, `ask_daemon`, `dismiss_daemon`. Named sessions survive MCP restarts and resume with zero token cost. See [ARCHITECTURE.md](ARCHITECTURE.md).

### Stenographer — Zero-Cost Session Notes

Incremental transcript narrator running on local Ollama. The token gate hook fires every ~50K tokens, Stenographer reads only the new transcript bytes, narrates them locally, appends to the session log. No API calls. $0.00 per save. See [starter-kit/stenographer/](starter-kit/stenographer/).

### The Airlock — File Snapshot Safety Net

Silently snapshots every file before Claude edits it. Three protection levels: strict (blocks if backup stale), best-effort (always snapshots), copy (source files). Every edit reversible. Zero prompts.

### Oracle Engine — Persistent Knowledge Daemons

Extends the daemon pattern into managed knowledge repositories. 17 tools covering oracle lifecycle, corpus management, health monitoring, and checkpoint/salvage/reconstitute. A single Gemini daemon holding your full codebase in its 2M-token window, queryable across sessions. See [docs/oracle-engine.md](docs/oracle-engine.md).

### Lifecycle Hooks

12 Claude hooks + 3 Gemini hooks + 2 Codex hooks covering the full session lifecycle: start, compaction, tool use, save, and recovery. See [docs/hooks/](docs/hooks/) for the complete reference.

### Core Skills

8 Claude skills encoding operating discipline — inter-agent protocol, context verification, documentation standards, systematic debugging, persistence enforcement, file taxonomy, failure crystallization, orchestrator-not-compute. See [starter-kit/README.md](starter-kit/README.md).

### Session Log Spec

A cross-agent session log standard so every agent documents its work in a format all three can read and resume from. See [SESSION_LOG_SPEC.md](SESSION_LOG_SPEC.md).

---

## System Requirements

| Requirement | Minimum | Notes |
|-------------|---------|-------|
| **Node.js** | v20+ | For building the MCP server |
| **OS** | macOS or Linux | Windows via WSL2 |
| **Claude Code** | Latest | Primary orchestrator — `claude` in PATH |
| **Gemini CLI** | Latest | Research + context agent — `gemini` in PATH |
| **Codex CLI** | Latest | Code agent — `codex` in PATH |
| **Ollama** | Latest | Required for Stenographer — `ollama pull qwen2.5:7b` |
| **jq, git** | Any | Used by hooks |
| **RAM** | 8 GB+ | Ollama models run locally |
| **Storage** | ~5 GB | Ollama model + npm dependencies |

---

## Quick Start

```bash
git clone --recurse-submodules https://github.com/michaeljboscia/triumvirate
cd triumvirate/starter-kit
chmod +x install.sh && ./install.sh
```

The installer walks you through setup interactively:
1. Copies hooks, skills, and rules to `~/.claude/`, `~/.codex/`, `~/.gemini/`
2. Builds the MCP server and wires all three agents' configs
3. Installs Stenographer with your choice of Ollama model
4. Creates `~/.ai-memory/` (git-initialized session log store)
5. Safe to re-run — backs up existing files before overwriting

### Post-Install

```bash
# Add credentials (DO NOT cp — the installer already created this file)
# Edit the existing .env and add your API keys
nano ~/.claude/.env
# At minimum: set GEMINI_API_KEY for pre-compact summarization

# Start working
cd ~/your-project && claude
```

See [docs/configuration-reference.md](docs/configuration-reference.md) for all environment variables and settings.

---

## Agent Setup Guide

> **You are an AI agent.** A human has asked you to install Triumvirate. This section is written for you. Follow these steps exactly.

### Step 1: Clone and Install

```bash
git clone --recurse-submodules https://github.com/michaeljboscia/triumvirate
cd triumvirate/starter-kit
chmod +x install.sh && ./install.sh
```

The install script is interactive — it will ask the user for their project directory, Ollama model preference, and subscription tier. **Do not run in background.** Let the user interact with it.

### Step 2: Build the MCP Server

If the installer didn't build it (or you need to rebuild):

```bash
cd triumvirate/mcp-server
npm install && npm run build
```

**Success signal:** Clean TypeScript compilation with zero errors.

### Step 3: Verify

After install, these MCP tools should be available in your next session:

| Tool | What It Does |
|------|-------------|
| `spawn_daemon` | Start a persistent Gemini or Codex session |
| `ask_daemon` | Query a running daemon (full conversation history) |
| `dismiss_daemon` | End a session (soft = resumable, hard = permanent) |
| `list_daemons` | Show all active daemon sessions |
| `send_message` | Fire-and-forget async message to another agent |
| `get_response` | Poll for async message response |
| `write_scratchpad` | Write to shared inter-agent scratchpad |
| `list_scratchpad` | Read shared scratchpad entries |
| `code_review` | Dispatch a structured code review |

If tools are missing, the MCP server config may not have been written. Check `~/.claude.json` for the `inter-agent` server entry.

### Step 4: Verify Ollama (for Stenographer)

```bash
ollama list | grep qwen2.5
```

If no model appears: `ollama pull qwen2.5:7b`

### Agent Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| MCP tools not available | Config not written to `~/.claude.json` | Run `install.sh` again, or manually add server entry |
| `spawn_daemon` fails | Gemini/Codex CLI not in PATH | `which gemini && which codex` — install missing CLIs |
| Daemon timeout on `ask_daemon` | Model quota exhausted | Fallback chain handles this automatically; check `~/.gemini/quota-state.json` |
| Stenographer not firing | Ollama not running | `ollama serve` in a separate terminal, then `ollama pull qwen2.5:7b` |
| Session logs not saving | `AI_MEMORY_DIR` not set or not a git repo | `mkdir -p ~/.ai-memory && cd ~/.ai-memory && git init` |
| `npm run build` fails | Missing Node.js 20+ | `node --version` — install/upgrade if needed |
| Hooks not firing | `settings.json` not updated | Re-run `install.sh` or copy from `starter-kit/claude/settings.json` |

---

## Session Logs

Session logs are AI working memory — they don't belong inside project repos. They go to a dedicated private memory store:

```
~/.ai-memory/                          # or $AI_MEMORY_DIR
└── my-project/
    ├── ..._v1_gemini.md
    ├── ..._v1_codex.md
    └── ..._v80_claude.md
```

Three agents, three logs, one shared directory. Any agent can read any other agent's log to pick up context across sessions. Daemons write logs automatically on dismiss.

See [SESSION_LOG_SPEC.md](SESSION_LOG_SPEC.md) for the naming convention, required sections, and cross-agent compatibility rules.

---

## Security Posture

Both Gemini and Codex CLIs apply sandboxes by default. Triumvirate disables these for the MCP daemon context:

- **Codex:** `--dangerously-bypass-approvals-and-sandbox`
- **Gemini:** `--approval-mode yolo`

These are the CLIs' own documented escape hatches for programmatic environments. The MCP server controls what prompts reach the agents. Only use Triumvirate in trusted contexts on your own machine.

---

## Ecosystem

| Project | What It Does |
|---------|-------------|
| [Pythia](https://github.com/michaeljboscia/pythia) | Local code search + architectural memory for AI agents (MCP server, runs on your machine) |
| [Claude Deep Research](https://github.com/michaeljboscia/claude-deep-research) | Batch-submit research topics to claude.ai's Deep Research via browser automation |

---

## License

[FSL-1.1-ALv2](LICENSE) — Free for internal use, education, and research. Commercial competing use prohibited for 2 years per release, then auto-converts to Apache 2.0.

---

## Contributing

Issues and PRs welcome. The agents will review your code — that's not a joke, it's the workflow.

---

## Acknowledgements

Operational resilience patterns — rate limit retry, stale worker detection, structured result contracts — were inspired by [CodeFleet](https://github.com/techinfobel/codefleet) by [techinfobel](https://github.com/techinfobel). CodeFleet takes a pipeline/DAG approach; Triumvirate takes a conversational daemon approach. Different tools for different problems.
