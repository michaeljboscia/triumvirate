# Triumvirate Architecture

## The core primitive: daemon sessions

Everything in Triumvirate is built on one pattern:

```
spawn_daemon(cwd)   →  daemon_id
ask_daemon(id, q)   →  answer
dismiss_daemon(id)  →  session cleaned up
```

Each daemon is a persistent AI session. You spawn it, ask it questions across multiple turns (full conversation history maintained), and dismiss it when done.

> **Roadmap:** Automatic session log writing on `dismiss_daemon` is in progress — see `SESSION_LOG_SPEC.md` for the format and the main Roadmap for implementation status.

Under the hood: each `ask_daemon` is a fresh subprocess (~2-3s for Gemini, ~7s for Codex) that resumes the conversation via the CLI's native session continuity. No PTY, no long-running process, no sentinel protocol.

---

## Topologies

### Star (default)

```
        Claude (orchestrator)
       /                     \
  Gemini                    Codex
  (context, research)       (code review, generation)
```

Claude decides what to delegate, collects results, synthesizes. Best for:
- Peer review workflows (dispatch both twins simultaneously)
- Research + analysis pipelines
- When you want Claude to control the flow

### Triangle (delegated pair)

```
Claude ──────► Codex (autonomous)
                  │
                  ▼
               Gemini (sub-resource)
```

Claude dispatches Codex with a large task. Codex decides autonomously whether to spin up a Gemini daemon for large-file context. Claude just waits for the finished result.

Best for:
- Large codebase reviews (file >500 lines)
- Tasks where Codex needs full-file context it shouldn't load into its own context window
- Reducing Claude's context window pressure

**Token economics:** Codex asks Gemini targeted questions. Gemini returns focused answers. Neither passes file content — only paths. The 4000-line file exists once, in Gemini's 2M-token context.

---

## The MCP server

Two MCP servers, one per sibling agent:

```
mcp-server/
├── src/gemini/      →  wraps `gemini` CLI
├── src/codex/       →  wraps `codex` CLI
└── src/shared/      →  context detection, session logs, job store, scratchpad
```

Both expose the same interface:
- `spawn_daemon` — start a persistent session
- `ask_daemon` — send a question, get an answer (blocking, with heartbeat logging)
- `dismiss_daemon` — clean up (session log writing: roadmap)
- `send_message` / `get_response` — fire-and-forget async (for one-shot requests)
- `list_daemons`, `list_jobs`, `list_scratchpad`, `write_scratchpad` — housekeeping

The shared utilities handle:
- **Context detection** (`context-detector.ts`) — infers project taxonomy from `taxonomy.json` or git remote
- **Session log finder** (`session-log-finder.ts`) — locates the most recent log per agent
- **Job store** (`job-store.ts`) — async job lifecycle for `send_message` pattern
- **Scratchpad** (`scratchpad-reaper.ts`) — shared filesystem scratch space between agents
- **Outbox logger** (`outbox-logger.ts`) — records all inter-agent messages for audit

---

## Session logs as shared memory

The key architectural insight: **session logs are shared memory**.

Each agent writes its work to `session-logs/` in the project directory, using a common format (`SESSION_LOG_SPEC.md`). Any agent can read any other agent's log to pick up context — especially useful when handing off between sessions or recovering from compaction.

```
project/session-logs/
  owner--client_domain_repo_feature_20260228_v1_gemini.md   ← what Gemini did
  owner--client_domain_repo_feature_20260228_v1_codex.md    ← what Codex did
  owner--client_domain_repo_feature_20260228_v80_claude.md  ← what Claude did
```

Logs are written:
- **Automatically** — by `dismiss_daemon` *(roadmap — not yet implemented)*
- **Manually** — via `/session-notes` skill in Claude Code
- **On compaction** — by pre-compact hooks (Tier 2)

The spec (`SESSION_LOG_SPEC.md`) defines sections, naming convention, versioning, and git workflow so logs from all three agents are compatible and mergeable.

---

## Sandbox notes

Both sibling CLIs apply their own sandboxes by default:

| CLI | Default sandbox | Problem | Fix applied |
|-----|----------------|---------|-------------|
| Gemini | Restricted to temp directory | Can't read project files | `--approval-mode yolo --include-directories ~` |
| Codex | `workspace-write` seatbelt | Blocks outbound TCP (LAN, SSH) | `--dangerously-bypass-approvals-and-sandbox` |

These flags are already applied in the MCP server. Both are the CLIs' own documented escape hatches for controlled environments.

---

## Tier 2: hooks (advanced)

The hooks system adds session persistence that survives context compaction:

```
Claude session running
  → Context approaching limit
  → pre-compact.sh fires
  → Extracts conversation as structured event log
  → Sends to Gemini CLI for summarization
  → Gemini writes session log to session-logs/
  → Git commits the log
  → Compaction happens
  → post-compact-recovery.sh fires
  → Reads the Gemini summary back into context
```

Result: Claude "remembers" across compaction boundaries. The session log is the memory.

Same pattern exists for Codex and Gemini — each has a pre-compact hook that writes its own log.

This requires: Claude Code hooks configured, Gemini CLI available locally, Codex hooks configured. Full implementation in `docs/hooks/`.

---

## Design decisions

**Why not a relay server?**
Zero infrastructure. Every component runs as a local MCP stdio process. Nothing to deploy, nothing to keep running, nothing to debug at 2am.

**Why stateful daemons instead of pub-sub?**
Conversation history matters. A code reviewer that can remember "you asked me about auth earlier" produces better results than one that starts fresh every message.

**Why pass paths, not content?**
All three CLIs can read files directly from disk (sandboxes unblocked). Passing file content between agents doubles the token cost and fills context windows with data that could be read once, fresh.

**Why session logs in git?**
Git is already there. It's append-only, diffable, and every engineer knows it. Adding a vector database or external memory service would add infrastructure and a new failure mode.
