# Configuration Reference

Every environment variable, config file, and setting in Triumvirate.

---

## Environment Variables

### Required

| Variable | Where Set | Purpose |
|----------|-----------|---------|
| `GEMINI_API_KEY` | `~/.claude/.env` | Gemini API access. Required for pre-compact summarization, oracle engine, and all Gemini daemon operations. |

### Optional (with defaults)

| Variable | Default | Purpose |
|----------|---------|---------|
| `AI_MEMORY_DIR` | `~/.ai-memory` | Root directory for cross-agent session logs. Must be a git repo. Falls back to `<cwd>/session-logs/` if not set. |
| `PYTHIA_HOME` | `~/.pythia` | Root directory for oracle registry, state files, and manifests. |
| `PYTHIA_REGISTRY_PATH` | `~/.pythia/registry.json` | Path to the oracle registry file. Override if you want the registry elsewhere. |
| `GEMINI_CLI_PATH` | `gemini` | Path to the Gemini CLI binary. Only needed if `gemini` is not in your PATH. |
| `CODEX_CLI_PATH` | `codex` | Path to the Codex CLI binary. Only needed if `codex` is not in your PATH. |
| `SESSION_LOG_SPEC_PATH` | (empty) | Path to SESSION_LOG_SPEC.md. Used by Gemini tools for session log writing. If empty, daemons produce session logs without the spec reference. |
| `TOKEN_GATE_THRESHOLD_KB` | `50` | Transcript size threshold (in KB) before the token-gate hook triggers Stenographer. Lower = more frequent saves. |
| `STENOGRAPHER_MODEL` | `qwen2.5:32b` | Ollama model used by Stenographer for session note generation. Alternatives: `qwen2.5:14b` (faster, less quality), `qwen2.5:7b` (fastest). |
| `STENOGRAPHER_TIMEOUT` | `300` | Maximum seconds for a single Stenographer save. Kills the process if exceeded. |
| `STENOGRAPHER_MAX_CHARS` | `5000` | Maximum characters sent to Ollama per save. Truncates if exceeded. |

### Hook-Specific Variables

| Variable | Default | Used By | Purpose |
|----------|---------|---------|---------|
| `TRIUMVIRATE_DEBUG` | (unset) | All hooks | Set to `1` to enable verbose debug output from hooks. |
| `ARTIFACT_GUARD_DIR` | `~/.claude/artifact-guard` | `pre-tool-use-artifact-guard.sh` | Directory for file snapshots. |
| `ORACLE_PRESSURE_INTERVAL` | `5` | `post-tool-use-oracle-pressure.sh` | Check pressure every N tool calls. |
| `MODE_NUDGE_TOOL_THRESHOLD` | `15` | `post-tool-use-mode-nudge.sh` | Suggest execution mode after N tool calls. |
| `MODE_NUDGE_TIME_THRESHOLD` | `1200` | `post-tool-use-mode-nudge.sh` | Suggest execution mode after N seconds. |

---

## Config Files

### `~/.claude/settings.json`

**What it is:** Claude Code's main settings file. Controls permissions and hooks.

**Installed from:** `starter-kit/claude/settings.json`

**Structure:**
```json
{
  "permissions": {
    "allow": [],     // Tools that run without asking
    "deny": [],      // Tools that are blocked
    "ask": [],       // Tools that prompt for approval
    "defaultMode": "acceptEdits"  // Default permission mode
  },
  "hooks": {
    "SessionStart": [...],    // Fires when session starts
    "PreCompact": [...],      // Fires before context compaction
    "PreToolUse": [...],      // Fires before any tool call
    "PostToolUse": [...]      // Fires after any tool call
  },
  "preferences": {
    "verboseThinking": true   // Enable extended reasoning
  }
}
```

**Hook matchers:** Each hook entry has a `matcher` that filters which tools trigger it:
- `"*"` — all tools
- `"Edit|Write"` — only Edit and Write tools
- `"Bash"` — only Bash tool
- `"mcp__supabase__apply_migration|mcp__supabase__execute_sql"` — specific MCP tools
- `"compact"` — special: fires after context compaction (not a tool)

### `~/.claude/settings.local.json`

**What it is:** Local overrides for settings.json. Not committed to repos. Primarily used for oracle permissions.

**Installed from:** `starter-kit/claude/settings.local.json.example` (manual copy)

### `~/.claude.json`

**What it is:** Claude Code's MCP server registration file.

**Installed by:** `install.sh` (merges inter-agent servers into existing config)

**Structure:**
```json
{
  "mcpServers": {
    "inter-agent-gemini": {
      "command": "/path/to/triumvirate/mcp-server/start-gemini.sh"
    },
    "inter-agent-codex": {
      "command": "/path/to/triumvirate/mcp-server/start-codex.sh"
    }
  }
}
```

### `~/.claude/.env`

**What it is:** Credential vault. Sourced by `session-start.sh` on every session.

**Installed from:** `starter-kit/shared/.env.example` (manual copy + fill in values)

**Never commit this file. Never display its contents.**

### `.claude/taxonomy.json` (per-project)

**What it is:** Project identity. Every project needs one.

**Structure:**
```json
{
  "owner": "your-github-username",
  "client": "client-or-org-name",
  "domain": "infrastructure|tooling|frontend|etc",
  "repo": "repository-name",
  "feature": "current-feature-branch"
}
```

**Used by:** Session log naming, hook behavior, inter-agent routing, agent log path computation.

**Fallback chain (if taxonomy.json missing):**
1. Try `.claude/taxonomy.json`
2. Try git remote URL parsing (`github.com/owner/repo`)
3. Fall back to directory name
4. Fall back to `"unknown"`

### `~/.codex/config.toml`

**What it is:** Codex CLI configuration.

**Installed from:** `starter-kit/codex/config.toml`

**Key section:**
```toml
[mcp_servers.inter-agent-gemini]
command = "/path/to/triumvirate/mcp-server/start-gemini.sh"
```

### `~/.gemini/settings.json`

**What it is:** Gemini CLI's settings file.

**Key section:**
```json
{
  "mcpServers": {
    "inter-agent-codex": {
      "command": "/path/to/triumvirate/mcp-server/start-codex.sh"
    }
  }
}
```

---

## Directory Structure

### Runtime directories (created by install.sh and hooks)

```
~/.claude/
├── hooks/              ← All Claude hook scripts
├── skills/             ← Skill definitions (SKILL.md files)
├── rules/              ← Auto-loading rule files
├── lessons/            ← Real-time lesson capture
├── settings.json       ← Claude Code settings + hooks
├── settings.local.json ← Oracle permissions (optional)
├── CLAUDE.md           ← Behavioral contract
├── .env                ← Credentials (never commit)
└── artifact-guard/     ← File snapshots from The Airlock

~/.codex/
├── hooks/              ← Codex hook scripts
├── skills/             ← Codex skill definitions
├── config.toml         ← Codex CLI config + MCP registration
└── AGENTS.md           ← Codex behavioral instructions

~/.gemini/
├── hooks/              ← Gemini hook scripts
├── settings.json       ← Gemini CLI settings + MCP registration
├── GEMINI.md           ← Gemini behavioral instructions
├── daemon-sessions/    ← Per-daemon working directories (auto-created)
└── quota-state.json    ← Model fallback quota tracking (auto-created)

~/.triumvirate/
├── stenographer/       ← Stenographer Python scripts
│   ├── stenographer.py
│   ├── parsers/
│   ├── prompts/
│   ├── session-save-ctl.py
│   └── session-save-worker.py
└── locks/              ← Lock files for concurrent operation prevention

~/.pythia/              ← Oracle engine state (auto-created)
├── registry.json       ← All oracles across all projects
├── oracles/            ← Per-oracle state + manifests
└── logs/               ← Oracle operation logs

~/.ai-memory/           ← Cross-agent session log store (git repo)
└── <project-name>/     ← Session logs per project
    ├── owner--client_domain_repo_feature_YYYYMMDD_vN_claude.md
    ├── owner--client_domain_repo_feature_YYYYMMDD_vN_gemini.md
    └── owner--client_domain_repo_feature_YYYYMMDD_vN_codex.md
```

---

## Customization Points

### Adding a new hook

1. Write the script in your project or `~/.claude/hooks/`
2. Add a hook entry to `~/.claude/settings.json`:
```json
{
  "matcher": "ToolName|OtherTool",
  "hooks": [
    {
      "type": "command",
      "command": "~/.claude/hooks/your-hook.sh"
    }
  ]
}
```
3. The script receives JSON on stdin with the tool call context

### Adding a new skill

1. Create a directory: `~/.claude/skills/your-skill/`
2. Create `SKILL.md` with frontmatter:
```markdown
---
name: your-skill
description: When to use this skill (1-2 sentences). Claude reads this to decide when to load it.
---

# Your Skill Title

[Content — rules, checklists, patterns, examples]
```
3. Claude will see the skill in its available skills list and load it when the description matches the task

### Adding a new rule

1. Create a `.md` file in `~/.claude/rules/`
2. Rules auto-load based on file globs — when Claude touches files matching certain patterns, the relevant rules appear
3. Keep rules focused: one concern per file

### Adding files to an oracle corpus

```
oracle_add_to_corpus({
  oracle_id: "your-oracle-id",
  file_path: "/absolute/path/to/file.md",
  role: "reference"  // or "context" or "training"
})
```
Then sync: `oracle_sync_corpus({ oracle_id: "your-oracle-id" })`

### Changing the Stenographer model

Set `STENOGRAPHER_MODEL` in `~/.claude/.env`:
```
STENOGRAPHER_MODEL=qwen2.5:14b   # Faster, good enough for most
```

Or pull a different Ollama model entirely:
```bash
ollama pull llama3.2:8b
# Then set STENOGRAPHER_MODEL=llama3.2:8b
```

### Disabling a hook

Remove its entry from `~/.claude/settings.json`. The script stays on disk but won't fire.

### Running without the oracle

Remove the `registerOracleTools(server)` line from `mcp-server/src/gemini/server.ts` and rebuild (`npm run build`). Everything else works unchanged.

### Running without Stenographer

Don't install Ollama. The token-gate hook will detect that Ollama is missing and skip saves silently. Session persistence still works through the pre-compact hook (which uses Gemini CLI, not Ollama).

### Running without Codex

Remove the `inter-agent-codex` entry from `~/.claude.json`. Claude-to-Gemini communication works independently.

### Running without Gemini

Not recommended — many hooks depend on Gemini CLI for summarization. If you must, the hooks will fall back to `jq`-based summarization (less quality but functional).
