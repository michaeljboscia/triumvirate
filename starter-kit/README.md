# Triumvirate Starter Kit

One-command setup for the complete multi-agent operating environment.

## What You Get

### Claude Code Hooks (8 files)
| Hook | Event | Purpose |
|------|-------|---------|
| `session-start.sh` | SessionStart | **Project Picker** when in HOME, **Session Recovery** when in a project |
| `post-compact-recovery.sh` | SessionStart:compact | Restores context after memory compaction |
| `pre-compact.sh` | PreCompact | Auto-saves session state before compaction (Gemini summarization → git commit) |
| `post-tool-use.sh` | PostToolUse | Auto-stages files, logs activity, taxonomy enforcement |
| `post-tool-use-token-gate.sh` | PostToolUse | Auto-saves at configurable token thresholds (~50K tokens) via Stenographer |
| `pre-tool-use-artifact-guard.sh` | PreToolUse | **The Airlock** — Snapshots every file before edit; enforces backup requirements for Supabase SQL |
| `pre-tool-use-bash-guard.sh` | PreToolUse | Blocks destructive SQL (DELETE/TRUNCATE/DROP) without a fresh backup |
| `_find-session-log.sh` | (shared) | Helper that finds the latest session log across multiple locations |

### Codex CLI Configuration
- `config.toml` — Model, approval mode, hooks, MCP server wiring
- `hooks/session-start.sh` — Session log recovery for Codex
- `hooks/pre-compact.sh` — Auto-save before Codex context compaction
- `skills/send-to-claude/` — Send inter-agent messages to Claude
- `skills/save-session/` — Save Codex session logs in shared format

### Gemini CLI Configuration
- `GEMINI.md` — Starter instructions for Gemini as a Triumvirate member
- `hooks/session-start.sh` — Session log recovery for Gemini
- `hooks/pre-compact.sh` — Self-summarization before context compaction (Gemini summarizes its own transcript)
- `hooks/post-tool-use.sh` — Auto-stages files, logs activity to session log

### Stenographer (Local Session Notes Engine)
- `stenographer.py` — Main orchestrator: state management, lock, delta extraction, Ollama generation
- `parsers/claude.py` — Claude JSONL byte-range parser with secret redaction
- `parsers/gemini.py` — Gemini JSON message-index parser
- `parsers/codex.py` — Codex JSONL byte-range parser
- `prompts/incremental.txt` — Incremental summarization prompt template
- `prompts/gapfill.txt` — Gap-fill prompt for missed content

Runs locally via Ollama — **zero API cost, zero context window impact**. Called automatically by the token gate hook. See [`stenographer/README.md`](stenographer/README.md) for details.

### Shared Templates
- `.env.example` — Credential vault template (API keys)
- `taxonomy.json.example` — Project taxonomy template

## Quick Start

```bash
# 1. Clone the repo
git clone https://github.com/michaeljboscia/triumvirate
cd triumvirate/starter-kit

# 2. Run the installer
chmod +x install.sh
./install.sh

# 3. Set up your credentials
cp ~/.claude/.env.example ~/.claude/.env
# Edit ~/.claude/.env with your API keys (at minimum: GEMINI_API_KEY)

# 4. Uncomment .env sourcing in session-start.sh
# Edit ~/.claude/hooks/session-start.sh — uncomment the .env block (~line 13)

# 5. Create your first project — it MUST be a git repo
mkdir -p ~/projects/my-project/.claude
cd ~/projects/my-project
git init
cp ~/.claude/taxonomy.json.example .claude/taxonomy.json
# Edit .claude/taxonomy.json with your project details
git add .claude/taxonomy.json && git commit -m "init: add taxonomy"

# 6. Start working
claude
```

## Git Is Foundational — Not Optional

**Every project you work in must be a git repository.** The entire persistence model depends on it.

Here's what happens without git:
- `post-tool-use.sh` won't auto-stage your file changes (it calls `git add`)
- `pre-compact.sh` won't commit session logs before compaction (they disappear)
- Session logs still get written to disk, but they're not versioned or backed up
- After a context compaction, the agent wakes up with no memory of what happened

The hooks fail gracefully without git — they won't crash — but you lose the entire reason to use them.

**The flow:**
1. You edit a file → The Airlock snapshots it, `post-tool-use.sh` stages it with `git add`
2. You commit → `post-tool-use.sh` logs the commit hash to the session log
3. Claude hits ~50K tokens → `token-gate.sh` runs `pre-compact.sh` in the background → session log written and **committed to git**
4. Claude runs out of context → `pre-compact.sh` runs synchronously → Gemini summarizes the transcript → session log written and **committed to git**
5. New session starts → `session-start.sh` finds the latest session log → reads it → injects context → you pick up where you left off

Git is the persistence layer. Session logs in git = AI memory that survives across sessions, machines, and agents.

---

## How Session Logs Work

Session logs are markdown files that serve as shared memory across all three agents.

### Naming Convention

```
<owner>--<client>_<domain>_<repo>_<feature>_<YYYYMMDD>_v<N>_<agent>.md
```

Example: `mikeboscia--personal_infrastructure_triumvirate_hooks_20260228_v3_claude.md`

The `<agent>` suffix (`_claude`, `_codex`, `_gemini`) tells you which agent wrote the log. All three agents search for session logs across all suffixes — Claude will read a Gemini log, and vice versa.

### Where They Live

The hooks look for session logs in this order:
1. `$AI_MEMORY_DIR/<repo>/` — central memory store (default: `~/.ai-memory/<repo>/`)
2. `<project>/session-logs/` — project-local

If `~/.ai-memory/` exists, session logs go there (cross-project, easy to back up). Otherwise they go in `<project>/session-logs/`.

### When They're Created

| Trigger | Hook | What Happens |
|---------|------|-------------|
| Claude hits ~50K tokens | `post-tool-use-token-gate.sh` | Runs `pre-compact.sh` in background, writes + commits log |
| Claude runs out of context | `pre-compact.sh` (PreCompact event) | Gemini summarizes transcript → writes + commits log |
| New Codex session | `codex/hooks/pre-compact.sh` | Writes + commits a Codex session log |
| Manual request | Ask Claude to update the session log | Claude writes narrative update |

### What They Contain

The log written by `pre-compact.sh` has these sections (auto-generated):
- **GEMINI CONTEXT SUMMARY** — Gemini's structured summary of what happened (or jq fallback if Gemini unavailable)
- **TRANSCRIPT HISTORY** — table of all previous transcript UUIDs for this feature
- **SESSION ACTIVITY LOG** — timestamped actions and commits

### The Shared Memory Model

All three agents share a session log and can read each other's entries. When you escalate from Claude to Gemini using `/send-to-gemini`, the skill commits the current session log to git first, then tells Gemini where to find it. Gemini reads the same log, has full context, and writes its findings back — either to the same log or a sibling log (with `_gemini` suffix).

This is why the taxonomy naming matters — it's how any agent finds any other agent's logs.

---

## Prerequisites

- **jq** — Required by all hooks (`brew install jq` on macOS)
- **Claude Code** — The CLI itself ([claude.ai/claude-code](https://claude.ai/claude-code))
- **Gemini CLI** (optional) — For intelligent pre-compact summarization. Without it, hooks fall back to jq-based transcript extraction.
- **Codex CLI** (optional) — For multi-agent code generation. Requires the [hooks-enabled fork](https://github.com/michaeljboscia/codex).
- **git** — Used by hooks for auto-staging and session log commits
- **Ollama** (optional) — For Stenographer local session notes. Without it, token gate logs thresholds but doesn't generate notes. Install: `brew install ollama` then `ollama pull qwen2.5:32b`

## How the Hooks Work

```
SessionStart
  └─► session-start.sh
        ├─ HOME? → Project Picker (show list, user picks, cd there)
        └─ Project? → Read latest session log → inject context

Every Tool Call:
  └─► PreToolUse: The Airlock (pre-tool-use-artifact-guard.sh)
        ├─ Supabase SQL → Check backup freshness → ALLOW or DENY
        ├─ Edge functions, n8n workflows → Snapshot, always allow
        ├─ Source files, markdown → Snapshot, always allow
        └─ node_modules, .git, /tmp → No action (pass)
  └─► PreToolUse: bash-guard (pre-tool-use-bash-guard.sh)
        └─ DELETE/TRUNCATE/DROP/pg_restore → Check for fresh backup → ALLOW or DENY
  └─► [Tool executes]
  └─► PostToolUse: post-tool-use.sh
        └─ git add (staged = experimental) → on test pass: git commit (blessed)
  └─► PostToolUse: token-gate.sh
        └─ Transcript growth > threshold? → Background save via Stenographer (local Ollama)

Before Memory Loss:
  └─► PreCompact: pre-compact.sh
        └─ Extract transcript → Gemini Pro summarization → session log → git commit
  └─► SessionStart:compact: post-compact-recovery.sh
        └─ Read the summary back into context
```

## Configuration

### Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `GEMINI_CLI_PATH` | Path to Gemini CLI | `gemini` |
| `AI_MEMORY_DIR` | Central session log store | `~/.ai-memory` |
| `SUPABASE_BACKUP_TTL_MINS` | Backup freshness window | `30` min |
| `TOKEN_GATE_THRESHOLD_KB` | Auto-save threshold | `200` KB (~50K tokens) |
| `TOKEN_GATE_COOLDOWN_SECS` | Min time between saves | `300` sec |
| `TOKEN_GATE_DISABLE` | Disable token gate | `0` |
| `ARTIFACT_GUARD_BYPASS` | Emergency bypass for Airlock | `0` |
| `BASH_GUARD_BYPASS` | Emergency bypass for bash guard | `0` |
| `STENOGRAPHER_MODEL` | Ollama model for session notes | `qwen2.5:32b` |
| `STENOGRAPHER_TIMEOUT` | Ollama generation timeout (seconds) | `180` |
| `STENOGRAPHER_NUM_CTX` | Ollama context window size | `65536` |

### Taxonomy

Each project needs a `.claude/taxonomy.json`:

```json
{
  "owner": "your-username",
  "client": "personal",
  "domain": "infrastructure",
  "repo": "my-project",
  "feature": "main"
}
```

This drives session log naming: `owner--client_domain_repo_feature_YYYYMMDD_vN_agent.md`

All three agents (Claude, Codex, Gemini) read/write session logs in this format — they become shared memory.

## What the Installer Does

1. **Copies Claude hooks** (8 files) to `~/.claude/hooks/` (backs up existing)
2. **Merges hook config** into `~/.claude/settings.json` (or creates if absent)
3. **Installs CLAUDE.md** starter template (skips if you already have one)
4. **Copies Codex hooks + skills** to `~/.codex/`
5. **Installs config.toml** template for Codex (skips if exists)
6. **Copies Gemini hooks** (3 files) to `~/.gemini/hooks/`
7. **Installs GEMINI.md** starter template (skips if exists)
8. **Builds the inter-agent MCP server** (`npm install && npm run build`)
9. **Wires MCP configs** for all three agents (Claude → Gemini+Codex, Gemini → Codex, Codex → Gemini)
10. **Installs Stenographer** to `~/.triumvirate/stenographer/` (checks for Ollama + model)
11. **Copies .env.example** and **taxonomy.json.example** as reference
12. **Creates `~/.ai-memory/`** — git-initialized central session log store

Safe to re-run — always backs up before overwriting.

## Customization

After installation, customize:
- `~/.claude/CLAUDE.md` — Add your own Iron Laws and project instructions
- `~/.claude/settings.json` — Add tool permissions as needed
- `~/.codex/config.toml` — Configure MCP servers and model preferences
- `~/.gemini/GEMINI.md` — Add Gemini-specific instructions

The hooks themselves rarely need modification — they're configured via environment variables.
