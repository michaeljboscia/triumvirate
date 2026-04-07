# Stenographer Replacement Research — 2026-04-07

## Problem Statement

The custom stenographer (Python scripts + Gemini CLI + Ollama + bash hooks + JSON state file) has never reliably worked. Two major breaks in ~6 weeks. The architecture has too many moving parts, too many coordination flags, and zero health checks. It needs to be replaced with something that actually works.

## Candidate Comparison

### 1. claude-mem (STRONGEST CANDIDATE)

**What:** Open-source Claude Code plugin. SQLite + FTS5 + ChromaDB vector search. Bun worker service.

**Architecture:**
- Hooks into Claude Code lifecycle: SessionStart, PostToolUse, Stop, SessionEnd
- Every tool call captured as an "observation"
- Worker service compresses raw observations into structured summaries (title, narrative, facts, concepts, affected files)
- Session-level summaries auto-generated when Claude finishes responding
- SQLite at `~/.claude-mem/claude-mem.db` — single file, no coordination flags
- 3-layer progressive disclosure: lightweight index → detailed summaries → full raw data
- Registers `__IMPORTANT` MCP tool to guide retrieval pattern
- Web viewer UI at localhost:37777

**Why it's better than stenographer:**
- No JSON state file with mutable flags (the thing that deadlocked us)
- No Gemini CLI dependency (which can fail silently)
- No Ollama dependency (which requires a model pulled)
- SQLite is ACID — writes either succeed or fail, no half-states
- Worker processes observations per-tool-call, not in batches at compaction boundaries
- If the worker fails, observations queue — no data loss, no deadlock
- 10x compression ratio claimed
- Has its own health: web UI lets you SEE what was captured

**Concerns:**
- Uses Claude Agent SDK for compression (API cost per observation?)
- Bun dependency — one more runtime to maintain
- Unknown: does it handle multi-project isolation? (our setup has 15+ projects)
- Unknown: how does it interact with existing hooks?

**Source:** GitHub (santiagomed/claude-mem), claude-mem.ai, multiple Medium/DataCamp articles

---

### 2. memory-mcp (SIMPLE CANDIDATE)

**What:** MCP server that uses CLAUDE.md + `.memory/state.json`. Two-tier: compact CLAUDE.md for essentials, deeper state.json via MCP tools.

**Architecture:**
- Hooks into SessionEnd and PreCompact
- Writes essential context to CLAUDE.md (auto-loaded by Claude Code)
- Deeper recall via MCP tool calls to `.memory/state.json`
- No external dependencies beyond Node.js

**Why it might work:**
- Dead simple — leverages what Claude Code already does (read CLAUDE.md)
- No database, no worker service, no external APIs
- State is a JSON file in the project directory — git-committable
- If the MCP server crashes, CLAUDE.md still has the essentials

**Concerns:**
- CLAUDE.md bloat — our CLAUDE.md files are already complex
- JSON state file is... another JSON state file (the thing that broke the stenographer)
- No compression — raw context accumulates
- No vector search — retrieval is keyword-only or full-dump
- Less mature than claude-mem

**Source:** dev.to article on memory-mcp

---

### 3. Mem0 (ENTERPRISE CANDIDATE)

**What:** Production-grade memory layer. Hybrid datastore (graph + vector + key-value). Open source + cloud-hosted.

**Architecture:**
- MCP server wraps Mem0 Memory API
- Multi-level memory: user, session, agent
- Hybrid retrieval: graph traversal + vector similarity + key-value lookup
- Self-hosted or cloud-hosted
- Python SDK + REST API

**Why it might work:**
- Most sophisticated retrieval (graph + vector + KV)
- Built for production — benchmarked, documented
- Multi-agent support (relevant for triumvirate)
- MCP server means it works with Claude, Gemini, Codex clients

**Concerns:**
- Overkill? We need session notes, not a knowledge graph
- Cloud version = API calls = cost + latency
- Self-hosted = PostgreSQL + Qdrant/Chroma + Neo4j — heavy infrastructure
- MCP server repo is archived — shifted to cloud-hosted
- Different problem space: Mem0 is "remember user preferences across interactions," not "save session notes for later recovery"

**Source:** mem0.ai, GitHub (mem0ai/mem0), DataCamp, InfoWorld

---

### 4. mcp-memory-keeper (LIGHTWEIGHT MCP)

**What:** MCP server for persistent context. Stores AI context in `~/mcp-data/memory-keeper/`.

**Architecture:**
- Standard MCP server pattern
- Local file storage
- Claude Code calls MCP tools to save/retrieve context
- JSON snapshots, git-committable

**Why it might work:**
- Simple MCP server — no database, no worker
- Local storage — no external dependencies
- Git-friendly — snapshots can be committed

**Concerns:**
- Requires Claude to actively call MCP tools to save context — not automatic
- No hook-based automatic capture
- If Claude forgets to call the tool (or the instruction drifts), nothing gets saved
- No compression — raw JSON accumulates
- Less mature, smaller community

**Source:** GitHub (mcp-memory-keeper), towardsai.net

---

### 5. claude-session-tracker / claude-worktrace (LOGGING CANDIDATES)

**What:** Hook-based tools that log sessions to GitHub Issues, markdown files, or cloud storage.

**Architecture:**
- claude-session-tracker: creates GitHub Issue per session, logs prompts/responses as comments
- claude-worktrace: uses PreCompact/SessionEnd hooks, generates narrative worklog summaries
- Both use Claude Haiku/Sonnet for summarization

**Why they might work:**
- GitHub Issues = durable storage with built-in search, comments, labels
- Leverages infrastructure that already exists and is monitored
- Human-readable output (not JSON state files)

**Concerns:**
- GitHub API rate limits (generous but not unlimited)
- Haiku API cost per summary
- claude-worktrace: one person's project, unknown maintenance status
- Neither provides retrieval/injection — they log but don't re-inject context on session start

**Source:** Reddit, Hacker News, GitHub

---

## Recommendation

**Replace the stenographer with `claude-mem`.** Here's why:

1. **Automatic capture** — hooks into lifecycle events, no manual invocation needed
2. **SQLite storage** — ACID, single file, no mutable flags that can deadlock
3. **Per-observation processing** — doesn't batch at compaction boundaries where crashes cause data loss
4. **Progressive disclosure** — only loads what's needed, doesn't bloat context
5. **Built-in health visibility** — web UI shows what was captured
6. **Active project** — claude-mem.ai exists, multiple articles, community adoption

**Migration path:**
1. Install claude-mem (`npx claude-mem install`)
2. Verify it captures observations for 2-3 sessions
3. Remove stenographer hooks from `~/.claude/settings.json` (token-gate, pre-compact save logic)
4. Keep pre-compact for what it does well (Gemini gap-fill for compaction recovery)
5. Keep artifact-guard, lint, paralysis-guard — they're independent

**What we'd lose:**
- Google Drive sync (claude-mem stores locally in SQLite)
- Gemini-based summarization (claude-mem uses Claude API)
- Rolling session log format (replaced by SQLite observations)

**What we'd gain:**
- A system that actually works
- No more JSON state file deadlocks
- Per-tool-call granularity instead of batch-at-compaction
- Full-text search + vector search over session history
- Visual health check via web UI

## Open Questions

1. Does claude-mem handle multi-project isolation? (We work across 15+ repos)
2. What's the API cost for Haiku compression per observation?
3. Can we keep the pre-compact Gemini gap-fill as a fallback alongside claude-mem?
4. How does claude-mem interact with our existing PostToolUse hooks (lint, token-gate)?
5. Can we export claude-mem SQLite data to Google Drive for backup?
