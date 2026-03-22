# How It All Fits Together

This document explains every component of Triumvirate, why it exists, and how they connect. If you're new to this repo, start here.

---

## The Problem This Solves

You have three AI CLI agents on your machine: Claude Code, Gemini CLI, and Codex. Each has strengths — Claude orchestrates, Gemini holds 2M tokens and can search the web, Codex reviews and generates code. But they can't talk to each other. If Claude needs Gemini's opinion on a 4000-line file, you have to copy-paste between terminals. If Codex finds a bug, it can't tell Claude. You're the relay.

Triumvirate removes you from the relay loop. Claude spawns a Gemini daemon, asks it questions across multiple turns, and dismisses it when done — all through MCP tools. Codex can do the same. The agents coordinate directly, share context through session logs, and document their own work.

---

## The Layers

Triumvirate has four layers, each built on the one below it:

```
Layer 4: Skills (operating discipline)
Layer 3: Hooks (session lifecycle)
Layer 2: Stenographer (session persistence)
Layer 1: MCP Server (inter-agent communication)
```

### Layer 1: The MCP Server

**What it is:** A TypeScript MCP server that wraps the Gemini and Codex CLIs behind clean tool interfaces.

**Why it matters:** Without this, Claude can't programmatically talk to Gemini or Codex. You'd need bash scripts with shell escaping, JSON parsing, process management, and error handling. The MCP server handles all of that.

**Where it lives:** `mcp-server/src/`

**What it provides:**
- `spawn_daemon` / `ask_daemon` / `dismiss_daemon` — persistent multi-turn sessions
- `send_message` / `get_response` — fire-and-forget async requests
- `list_daemons`, `list_jobs` — housekeeping
- `write_scratchpad` / `list_scratchpad` — shared filesystem between agents
- `code_review` (Codex only) — git-aware code review
- `summarize_transcript` (Gemini only) — for pre-compact hooks
- 17 oracle tools (Gemini only) — persistent knowledge daemons (see `docs/oracle-engine.md`)

**How it works under the hood:**

Each `ask_daemon` spawns a fresh subprocess of the CLI (`gemini` or `codex`), passing `-r latest` to resume the existing conversation. The process runs for 2-7 seconds, produces output, and exits. There's no long-running process, no PTY, no sentinel protocol. Session continuity is provided by the CLIs themselves — they write conversation history to disk and resume it on the next invocation.

The daemon pattern works because both CLIs support session resumption:
- **Gemini:** `gemini -r latest -p "question" --output-format text` resumes the last conversation in the working directory
- **Codex:** `codex exec resume <thread_id> --json` resumes a specific thread

The MCP server creates a unique working directory per daemon (under `~/.gemini/daemon-sessions/` or equivalent) so sessions don't collide.

**Key design decision: paths, not content.** When Claude asks Gemini to read a file, it sends the file PATH, not the file content. All three CLIs can read files directly from disk. This means a 4000-line file exists once on disk — not duplicated in Claude's context window, then again in the MCP message, then again in Gemini's context window.

---

### Layer 2: The Stenographer

**What it is:** An incremental session note system that runs on local Ollama. Zero API cost.

**Why it matters:** AI conversations are ephemeral. When Claude's context window fills up, everything is compacted — your conversation history, architectural decisions, the state of in-progress work. Without session notes, the next session starts from zero. Stenographer writes a rolling narrative so the next session can pick up where you left off.

**Where it lives:** `starter-kit/stenographer/`

**How it works:**

```
1. You're working with Claude. The conversation grows.
2. Every ~50K tokens, the token-gate hook fires.
3. The hook calls stenographer.py.
4. Stenographer reads ONLY the new bytes from the transcript
   (it tracks a byte-offset cursor, so it never re-reads old content).
5. The parser (claude.py, gemini.py, or codex.py) converts
   the raw JSONL/JSON into normalized events.
6. These events are fed to a local Ollama model (e.g., qwen2.5:32b).
7. Ollama produces a plain-English paragraph summarizing the delta.
8. The paragraph is appended to the session log file.
```

**Why local Ollama instead of an API?** The original pre-compact hook piped full transcripts to the Gemini API for summarization. That burned 69 million tokens in 4 days. Stenographer costs $0.00 per save because it runs on your own machine.

**Why incremental?** A 200K-token conversation produces a ~50KB transcript. Re-reading the whole thing every save wastes compute. Stenographer reads only the new bytes since the last save — typically 5-10KB. This means saves are fast (2-5 seconds) and lightweight.

**The parsers:**
- `parsers/claude.py` — Reads Claude Code's JSONL transcript format. Uses byte-range cursors.
- `parsers/gemini.py` — Reads Gemini CLI's JSON array format. Uses message-index cursors.
- `parsers/codex.py` — Reads Codex's JSONL format. Uses byte-range cursors.

**The workers:**
- `session-save-ctl.py` — Controller for background save orchestration. Manages when saves happen, prevents concurrent saves, handles locks.
- `session-save-worker.py` — Worker process that performs the actual Ollama inference. Runs in background so the main session isn't blocked.

**Gap fill:** When compaction happens, there may be transcript content that Stenographer hasn't narrated yet (the gap between the last save and compaction). The `gap_fill.py` module handles this — it reads the remaining bytes, sends them to Gemini CLI (free tier) for a quick summary, and appends that to the session log.

---

### Layer 3: The Hooks

**What they are:** Shell scripts that fire on specific Claude Code (and Gemini/Codex) lifecycle events.

**Why they matter:** Without hooks, you'd have to manually save session notes, manually recover context after compaction, manually stage files, manually check for stale backups before editing. Hooks automate all of this. They're the connective tissue between the MCP server, Stenographer, and your daily workflow.

**Where they live:** `starter-kit/claude/hooks/`, `starter-kit/gemini/hooks/`, `starter-kit/codex/hooks/`

**The hook lifecycle:**

```
Session starts
  → session-start.sh fires (project picker or session recovery)
  → session-start-v3.sh fires (orphan cleanup)

You work...
  → Every Edit/Write: pre-tool-use-artifact-guard.sh fires (snapshots file)
  → Every Bash: pre-tool-use-bash-guard.sh fires (checks for destructive SQL)
  → Every Supabase MCP SQL: pre-tool-use-supabase-mcp-gate.sh fires (checks backup)
  → Every Edit/Write/Bash/Agent: post-tool-use.sh fires (auto-stages, logs activity)
  → Every tool call: post-tool-use-token-gate.sh fires (triggers Stenographer at ~50K)
  → Every tool call: post-tool-use-oracle-pressure.sh fires (checks oracle context)
  → Every tool call: post-tool-use-mode-nudge.sh fires (suggests execution mode)

Context is about to compact
  → pre-compact.sh fires (Gemini summarizes → session log → git commit)

After compaction
  → post-compact-recovery.sh fires (reads session log back into context)
```

#### Every hook explained:

**`session-start.sh`** (514 lines, SessionStart event)
The most complex hook. Has two modes:
- **HOME mode:** When you start Claude from your home directory, it presents a project picker — numbered list of all local projects with taxonomy, last session dates, and features. Also fetches GitHub repos not cloned locally. Shows recent session carousel (8 most recent across all projects). You type a number and it navigates you there.
- **Project mode:** When you start Claude inside a project directory, it finds the latest session log and injects it into context. This is how Claude "remembers" previous sessions.

Also handles: taxonomy resolution (reads `.claude/taxonomy.json` with fallback chain), global + project lessons injection, environment variable sourcing, Stenographer health check.

**`session-start-v3.sh`** (31 lines, SessionStart event)
Lightweight cleanup hook. On every session start, it runs `session-save-ctl.py --cleanup` to remove orphaned save files and stale lock files that might have been left behind by crashed sessions.

**`post-compact-recovery.sh`** (30 lines, SessionStart:compact event)
After compaction, Claude has lost its conversation history. This hook finds the most recent session log (written by pre-compact.sh moments before) and injects it as system context. The session log contains a Gemini-generated summary of everything that happened — architectural decisions, files modified, current state. Claude reads it and continues working.

**`pre-compact.sh`** (72 lines, PreCompact event)
Fires right before context compaction. Extracts the conversation as a structured event log, sends it to Gemini CLI for summarization, writes the summary as a session log, and git-commits it. This is the "save game" before the context window resets.

**`pre-tool-use-artifact-guard.sh`** (511 lines, PreToolUse:Edit|Write event)
**The Airlock.** Before every file edit, this hook silently snapshots the file to `~/.claude/artifact-guard/`. Three protection levels:
- `remote_strict` (Supabase SQL): Checks backup freshness + hash match. **Blocks the edit** if the backup is stale — you must take a fresh backup first.
- `remote_best_effort` (edge functions, n8n JSON): Always snapshots, always allows the edit. Provides rollback capability without blocking.
- `local_copy` (source code files): Always snapshots, always allows. Safety net for when Claude makes mistakes.

Why this matters: At 2am when Claude overwrites a Supabase migration with a broken one, you can recover from the snapshot. Every edit is reversible.

**`pre-tool-use-bash-guard.sh`** (331 lines, PreToolUse:Bash event)
Inspects bash commands before execution. If the command contains destructive SQL (`DROP`, `DELETE`, `TRUNCATE`, `ALTER`) targeting a remote database, and there's no fresh backup, it blocks the command. Prevents accidental data loss.

**`pre-tool-use-supabase-mcp-gate.sh`** (260 lines, PreToolUse:mcp__supabase__* event)
Same concept as bash-guard, but for Supabase MCP tools (`apply_migration`, `execute_sql`). These bypass bash entirely — they go through the Supabase MCP server. This hook catches them and applies the same backup-freshness check.

**`post-tool-use.sh`** (115 lines, PostToolUse:Edit|Write|Bash|Agent event)
After every edit, bash command, or agent dispatch:
- Auto-stages modified files in git (so changes are tracked immediately)
- Logs the action to the session activity table (file, lines changed, outcome)
This creates an audit trail of everything Claude did during a session.

**`post-tool-use-token-gate.sh`** (186 lines, PostToolUse:* event)
Checks the transcript size after every tool call. When it crosses ~50K tokens since the last save, it triggers Stenographer to write an incremental session note. This is the trigger mechanism for Layer 2.

**`post-tool-use-oracle-pressure.sh`** (150 lines, PostToolUse:* event)
Every 5 tool calls, checks if there are active oracle daemons and reports their context pressure. If an oracle is approaching its context limit, it emits a recommendation to checkpoint (save the oracle's knowledge before it degrades). This prevents oracle context exhaustion from sneaking up on you.

**`post-tool-use-mode-nudge.sh`** (106 lines, PostToolUse:* event)
After 15+ tool calls or 20+ minutes without an explicit execution mode set, suggests that you formalize the current work as an EXPLORE or EXECUTE mode. This is a soft prompt — it doesn't block anything, just nudges toward more disciplined workflow.

**`_find-session-log.sh`** (57 lines, shared helper)
Not a hook itself — a shared library sourced by other hooks. Delegates to `session_log_path.py` (the single source of truth for session log discovery) with legacy fallbacks.

---

### Layer 4: The Skills

**What they are:** Markdown documents that get loaded into Claude's system prompt when invoked. They encode operating discipline — rules, checklists, patterns, and failure modes that prevent common mistakes.

**Why they matter:** AI agents make the same mistakes repeatedly. Without skills, you'd correct Claude every session: "don't guess at codebase state — verify first," "don't skip persistence," "always use the inter-agent protocol." Skills encode these corrections once so they're enforced automatically.

**Where they live:** `starter-kit/claude/skills/`

**How skills work:** When Claude invokes a skill (via the `Skill` tool), the skill's content is loaded into the conversation. It's not executable code — it's structured guidance that Claude follows. Think of it as a runbook that gets injected into the AI's instructions.

#### Every skill explained:

**`inter-agent-protocol`**
The rules for how Claude talks to Gemini and Codex. Defines:
- The daemon pattern (`spawn_daemon` → `ask_daemon` → `dismiss_daemon`) as the primary communication method
- Pre-digest rules (read files yourself, send the key details inline — don't tell siblings "see the session log")
- Peer review gates (architectural changes require twin review from both Gemini AND Codex)
- Auto-escalation: after 3 consecutive failures on the same problem, STOP and escalate to both siblings
- Performance rules (Gemini search is native MCP, not inter-agent; use paths not content)

Why it's important: Without this, Claude defaults to verbose, context-wasteful communication patterns. It sends 4000-line files inline instead of paths. It skips peer review. It doesn't escalate when stuck.

**`context-before-action`**
The rule: before declaring anything about codebase state ("this function doesn't exist," "this file has no tests"), VERIFY by actually reading the code. Before planning architectural changes, load Gemini context with the relevant files.

Why it's important: AI agents confidently declare things that aren't true. "That function was removed" — except it's on a different branch. "There are no tests for this" — except they're in a test file with a non-obvious name. This skill gates action behind verification.

**`documentation-standards`**
DNA-level requirements for new capabilities: every new feature must have a README, schema definitions, example usage, and scripts. Defines the exact file structure and what each file must contain.

Why it's important: Without enforcement, features ship with zero documentation. The next session (or the next person) has no idea what was built or how to use it.

**`our-systematic-debugging`**
A debugging methodology: observe the failure, form hypotheses, test each one, find root cause, then fix. No guessing. No "let me try this and see if it works." Extends the base methodology with failure crystallization — when a bug is fixed, capture the pattern so it becomes a preventable class of error.

Why it's important: AI agents default to "let me try changing this" — they modify code speculatively without understanding the root cause. This leads to cascading fixes where each "fix" introduces a new bug. Systematic debugging prevents this.

**`persist-or-fail`**
The rule: every computation that produces results MUST save those results to persistent storage. No exceptions. No "I'll save it later." If the save fails, the operation fails.

Why it's important: Compute is expensive (time, tokens, API calls). If results aren't persisted, they evaporate when the session ends, the process crashes, or the context compacts. We lost 5 hours of batch processing once because results were held in memory and never written to disk. Never again.

**`file-taxonomy`**
A decision tree for where to put files in a project. Given a file type (config, script, doc, test, data), the taxonomy tells you exactly which directory it belongs in.

Why it's important: Without this, files end up in random locations. Config in the root, scripts in `src/`, data files mixed with code. The next session can't find anything because there's no consistent organization.

**`crystallize`**
A meta-system for converting recurring failures into enforceable rules. When you hit the same class of bug 3+ times, crystallize it: document the failure pattern, the root cause, the fix, and the prevention rule. The output becomes a new skill or skill matrix entry.

Why it's important: Failure patterns repeat. If you fix a bug but don't capture the pattern, you (or the AI) will make the same mistake again in a different context. Crystallization turns individual fixes into systemic prevention.

Includes sub-files:
- `factory/` — Templates for producing crystallized skills
- `reference/` — Example crystallized failures (real incidents)
- `enforcement.md` — How crystallized rules get enforced
- `validation.md` — How to verify a crystallized rule actually prevents the failure

**`orchestrator-not-compute`**
The rule: before writing a new script for a task, check if existing infrastructure already handles it. Don't re-implement what Prefect flows, Docker containers, or existing CLI tools already do. Claude's job is to orchestrate existing tools, not to be a compute engine.

Why it's important: AI agents love writing fresh code. They'll build a 200-line web scraper when there's already a Prefect flow that does the same thing. They'll manually browse GIS portals when there's a Docker container purpose-built for it. This skill forces a check before creation.

---

## The Config Files

### `settings.json` (starter-kit/claude/settings.json)
The Claude Code settings file. Defines:
- **Permissions:** Allow/deny/ask lists for tool access
- **Hooks:** Which scripts fire on which events (see Layer 3 above)
- **Preferences:** `verboseThinking: true` enables extended reasoning

This file is installed to `~/.claude/settings.json` by the installer.

### `settings.local.json.example` (starter-kit/claude/settings.local.json.example)
Oracle permission template. The oracle tools need explicit permission to run — this file pre-authorizes the 13 safe tools and puts the 3 destructive decommission tools behind manual approval.

### `mcp_config.json.example` (starter-kit/shared/mcp_config.json.example)
MCP server registration template. Shows how to wire the inter-agent Gemini and Codex servers into Claude Code's `~/.claude.json`. Without this, Claude can't see the MCP tools.

### `.env.example` (starter-kit/shared/.env.example)
Credential vault template. API keys for Gemini, GitHub, Supabase, etc. Sourced by session-start.sh on every session.

### `taxonomy.json.example` (starter-kit/shared/taxonomy.json.example)
Project taxonomy template. Every project needs a `.claude/taxonomy.json` that identifies it:
```json
{
  "owner": "your-username",
  "client": "client-name",
  "domain": "infrastructure",
  "repo": "project-name",
  "feature": "current-feature"
}
```
This taxonomy drives session log naming, hook behavior, and inter-agent routing.

### `CLAUDE.md` (starter-kit/claude/CLAUDE.md)
Claude's behavioral contract. Defines universal rules (fully qualified paths, commit early, lessons capture, completion mandate, 3-failure escalation), progressive disclosure gates (what to load before what), compaction recovery procedures, session persistence architecture, and sibling awareness.

### Rules (starter-kit/claude/rules/)
Three auto-loading rule files:
- `automation.md` — "Never use n8n, Make, or Zapier. Use Python + Prefect."
- `infrastructure.md` — "Zero trust by default. No 0.0.0.0/0 anywhere."
- `reporting.md` — "Never hardcode data. Every number comes from a live query."

Rules auto-load based on file globs — when you touch infrastructure files, the infrastructure rules appear automatically.

### Lessons (starter-kit/claude/lessons/TEMPLATE.md)
A template for capturing real-time lessons. Claude adds entries during sessions when it encounters errors, abandoned approaches, or non-obvious decisions. The format: context, what happened, the lesson, and what it applies to.

---

## The Oracle Engine (Layer 1.5)

The oracle sits between the basic MCP server and the hooks — it extends the daemon pattern into something more permanent.

**The problem it solves:** A basic daemon dies when you dismiss it or when the MCP server restarts. If you've loaded 50 research documents into a Gemini daemon, that context is gone. You'd have to re-feed everything in the next session.

**What the oracle does:** It manages a **corpus** (a manifest of files that define the oracle's knowledge), **state** (interaction history, degradation signals), and **lifecycle** (spawn, checkpoint, salvage, reconstitute, decommission). An oracle can:

1. **Survive session boundaries** — `spawn_oracle` with a `session_name` resumes an existing session
2. **Checkpoint** — synthesize the oracle's accumulated context into a checkpoint document. This is a compressed representation of everything the oracle knows, produced by the oracle itself.
3. **Reconstitute** — when an oracle's context is exhausted, create a new generation (v→v+1) that starts fresh but inherits the checkpoint. The knowledge transfers; the context window resets.
4. **Salvage** — emergency checkpoint when a daemon dies unexpectedly. Recovers whatever is still accessible.
5. **Decommission** — permanent destruction with 7 validation gates including TOTP verification. This is intentionally hard to prevent accidental deletion.

**The 17 tools, grouped:**

| Tool | What it does | When to use it |
|------|-------------|---------------|
| `oracle_init` | Creates a new oracle — registry entry, manifest, initial state | Starting a new project oracle |
| `spawn_oracle` | Spawns or resumes the oracle daemon | Beginning of every session |
| `oracle_health` | Read-only check — corpus file hashes, daemon status, context pressure | Diagnostic, non-destructive |
| `oracle_refresh` | Re-hashes stale manifest entries | After files changed on disk |
| `oracle_sync_corpus` | Pushes changed files to the running daemon | After adding/modifying corpus files |
| `oracle_pressure_check` | Computes context usage and recommends action | Monitoring, triggered by hooks |
| `oracle_log_learning` | Records an interaction (consultation, feedback, sync event, session note) | After significant interactions |
| `oracle_checkpoint` | Asks the oracle to synthesize a checkpoint | Before context gets too full |
| `oracle_salvage` | Emergency checkpoint from a dead/degraded daemon | When something goes wrong |
| `oracle_reconstitute` | Creates a new generation from a checkpoint | After checkpoint, to reset context |
| `oracle_quality_report` | Analyzes degradation signals — staleness, pressure, error rates | Periodic quality check |
| `oracle_add_to_corpus` | Adds a file to the manifest (static entry) | Growing the oracle's knowledge |
| `oracle_update_entry` | Updates a manifest entry's hash/metadata | After file modification |
| `oracle_decommission_request` | Initiates decommission — returns time-limited token | Starting the destruction sequence |
| `oracle_decommission_cancel` | Cancels an active decommission request | Changed your mind |
| `oracle_decommission_execute` | Executes decommission — 7 validation gates | Permanent destruction |

**Error handling:** Every oracle tool returns `OracleResult<T>` — either `{ ok: true, data: T }` or `{ ok: false, error: { code, message, retryable, details } }`. There are 25 error codes covering daemon states (BUSY_QUERY, DEAD, QUOTA_EXHAUSTED), operation failures (CHECKPOINT_FAILED, LOCK_TIMEOUT), and decommission gates (TOKEN_EXPIRED, TOTP_INVALID).

**The runtime bridge:** Oracle tools don't directly manage Gemini daemons. They go through `OracleRuntimeBridge` — an interface that provides `spawnDaemon`, `askDaemon`, `dismissDaemon`, and `executeWithFallback`. This decoupling means oracle-tools.ts (4,946 lines) has zero knowledge of how Gemini processes are actually spawned and managed. That's all in `runtime.ts` (632 lines).

---

## The Install Process

`starter-kit/install.sh` (466 lines) handles the complete setup:

1. **Claude hooks** — copies all `.sh` files from `starter-kit/claude/hooks/` to `~/.claude/hooks/`, makes them executable
2. **Claude skills** — copies skill directories to `~/.claude/skills/`, skips existing ones
3. **Claude rules + lessons** — copies rule templates, creates lessons directory
4. **Claude settings** — merges hooks into existing `settings.json` if present, or installs fresh. Won't overwrite existing hooks config (you'd need to merge manually).
5. **Claude CLAUDE.md** — installs starter template. Won't overwrite existing.
6. **Codex hooks + skills + config** — same pattern for `~/.codex/`
7. **Gemini hooks + GEMINI.md** — same pattern for `~/.gemini/`
8. **MCP server build** — `npm install && npm run build` in `mcp-server/`
9. **MCP wiring** — registers `inter-agent-gemini` and `inter-agent-codex` in:
   - `~/.claude.json` (Claude Code's MCP config)
   - `~/.gemini/settings.json` (Gemini's MCP config)
   - `~/.codex/config.toml` (Codex's MCP config)
10. **Stenographer** — installs to `~/.triumvirate/stenographer/`, creates state/lock directories, checks for Ollama
11. **AI Memory** — creates `~/.ai-memory/` (git-initialized) for cross-session log storage
12. **Templates** — copies `.env.example` and `taxonomy.json.example`

The installer is safe to re-run — it backs up every file before overwriting with a timestamped suffix.

---

## The Shared Utilities

These TypeScript modules in `mcp-server/src/shared/` are used by both the Gemini and Codex servers:

**`cli-executor.ts`** (464 lines)
The process spawning engine. Provides `executeCli` (blocking, waits for result) and `spawnCliAsync` (non-blocking, returns immediately with a job ID). Handles:
- Timeout with grace period (SIGTERM → wait → SIGKILL)
- Process group kill (detached spawn, kills entire process tree including child processes)
- Progress callbacks (spawned, heartbeat, stdout_data, timeout, retry, done)
- Retry on transient failures

**`agent-log-path.ts`**
Computes session log paths from taxonomy. Given a project's taxonomy.json and the agent name, produces a path like `~/.ai-memory/my-project/owner--client_domain_repo_feature_20260322_v1_claude.md`. Handles version incrementing (finds existing logs, bumps version number).

**`context-detector.ts`**
Infers project context from the working directory. Reads `.claude/taxonomy.json` if present, falls back to git remote URL parsing, falls back to directory name.

**`session-log-finder.ts`**
Finds the most recent session log for a given agent. Searches `$AI_MEMORY_DIR/<repo>/` and `<cwd>/session-logs/` with the appropriate agent suffix.

**`job-store.ts`**
SQLite-backed (WAL mode) job lifecycle for the `send_message` / `get_response` async pattern. Each `send_message` creates a job; the response is stored when it arrives; `get_response` retrieves it.

**`scratchpad-reaper.ts`**
Manages the inter-agent scratchpad — a shared filesystem directory where agents can leave files for each other. The reaper cleans up old files to prevent unbounded growth.

**`outbox-logger.ts`**
Records every inter-agent message to an outbox log. Useful for debugging — you can see exactly what Claude sent to Gemini and when.

**`message-formatter.ts`**
Normalizes messages between agents. Handles encoding, truncation, and format conversion.

**`types.ts`**
Shared TypeScript types used across both servers: `ExecutionResult`, `DaemonSession`, `SpawnRequest`, etc.

---

## Model Fallback Chain

**What it is:** When Gemini returns a quota-exhaustion error for one model, the system automatically tries the next model in the chain.

**The chain:** `gemini-3-pro-preview` → `gemini-2.5-pro` → `gemini-3-flash-preview` → `gemini-2.5-flash`

**How it works:**
1. First call tries the best available model
2. If quota error detected, that model is marked as exhausted with a 1-hour TTL
3. Next call skips exhausted models and tries the next one
4. State persisted in `~/.gemini/quota-state.json`
5. After 1 hour, exhausted models become available again

**Where it lives:** `mcp-server/src/gemini/model-fallback.ts`

**Why it matters:** Without this, a single quota exhaustion kills all inter-agent communication for the rest of the hour. The fallback chain degrades gracefully — you might get a slightly less capable model, but work continues.

---

## Session Logs as Shared Memory

This is the key architectural insight: **session logs are the shared memory layer between all three agents and across all sessions.**

When Claude finishes a session, the pre-compact hook writes a session log. When Gemini is dismissed, it writes a session log. When Codex reviews code, the result is logged. All logs go to the same directory (`~/.ai-memory/<project>/`) in a compatible format (`SESSION_LOG_SPEC.md`).

The next session — regardless of which agent starts it — can read all previous logs. Claude can read what Gemini researched yesterday. Codex can read what Claude decided last week. The logs are the continuity layer.

**The naming convention:**
```
owner--client_domain_repo_feature_YYYYMMDD_vN_agent.md
```

Example: `michaeljboscia--core_infrastructure_triumvirate_backport_20260322_v1_claude.md`

The version number increments within the same day and feature. The agent suffix identifies who wrote it.

---

## What Someone Cloning This Gets

After `git clone` + `./install.sh`:

1. **Inter-agent communication** — Claude can spawn Gemini/Codex daemons, ask questions, get answers
2. **Oracle engine** — persistent knowledge daemons that survive sessions
3. **Session persistence** — Stenographer writes rolling notes, pre-compact saves full summaries, post-compact recovers context
4. **File safety** — The Airlock snapshots every edit
5. **SQL safety** — Bash guard and Supabase gate prevent destructive operations without backup
6. **Operating discipline** — 8 skills that prevent the most common multi-agent failure modes
7. **Auto-staging** — Every edit is immediately staged in git
8. **Token management** — Oracle pressure monitoring and Stenographer token gate
9. **Shared memory** — Cross-agent session logs in a git-initialized private repo
10. **Model resilience** — Automatic fallback when Gemini quota is exhausted

What they DON'T get (and shouldn't):
- Domain-specific skills (marketing, cold email, SEO — these are business-specific)
- Production credentials (the `.env.example` is a template)
- Personal session logs (those are in your private `~/.ai-memory/`)
- Custom hooks (scope guard, ClickUp integration — these are environment-specific)
