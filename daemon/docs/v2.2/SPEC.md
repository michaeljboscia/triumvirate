# Triumvirate v2.2 — The Accountability Release

**Author:** Claude (orchestrator), with Gemini + Codex twin review
**Date:** 2026-04-07
**Branch:** TBD (feat/v2.2-accountability)
**Status:** DRAFT — goat rodeo Round 1 complete, Round 2 in progress

---

## Vision

v2.2 is the release where everything the system does gets **recorded, verified, and remembered.** Session notes never silently vanish. Lessons persist with confidence decay. Agents can't approve their own work. Fleets coordinate real parallel execution. And a dashboard makes all of it visible.

The theme: **if it happened and you can't prove it, it didn't happen.**

---

## Architecture: Crate Segmentation

v2.2 adds 4 new crates to the workspace. Each crate owns one concern. No crate reaches into another's storage.

### Current Crates (unchanged)

| Crate | Concern |
|-------|---------|
| `shared-types` | DTOs — request/response structs, no logic |
| `daemon-core` | File I/O, home dir, dead-drop, token, queue registry |
| `mcp-bridge` | Env config, URL builders, agent command resolution |
| `mcp-tools` | Progress emitter, heartbeat, display helpers |
| `agent-adapter` | Protocol parsers (Gemini stream-json, Codex exec-json) |
| `agent-worker` | Worker lifecycle, session reuse, spawn/dismiss |
| `daemon-http` | HTTP client wrappers for daemon API |
| `fallback-outbox` | Dead-drop ticket management |
| `triumvirate` | Binary — MCP stdio bridge + HTTP daemon + CLI |

### New Crates (v2.2)

| Crate | Concern | Depends On |
|-------|---------|-----------|
| `ledger` | SQLite-backed persistence — sessions, lessons, outbox, health | `shared-types`, `agent-adapter` |
| `fleet` | Worktree management, task lists, parallel agent dispatch, merge | `shared-types`, `daemon-core`, `agent-worker`, `ledger` |
| `peer-review` | Cross-model approval gates, review protocol | `shared-types`, `ledger` |
| `dashboard` | Svelte 5 build artifacts, static asset serving | `shared-types` (build-time only) |

### Dependency Rules

- **REQ-001:** New crates MUST depend only on `shared-types` and crates listed in their dependency column. No crate may depend on `triumvirate` (the binary). No circular dependencies.
- **REQ-002:** The `ledger` crate MUST NOT depend on `daemon-core`. It owns its own SQLite database and file paths. `daemon-core` continues to own JSONL/JSON storage for backward compatibility during migration.
- **REQ-003:** The `fleet` crate MUST NOT shell out to `git` directly. It MUST use a `GitOps` trait defined in `shared-types` so the binary provides the real implementation and tests provide a mock.
- **REQ-004:** The `dashboard` crate MUST produce static HTML/CSS/JS at build time. The `triumvirate` binary embeds these via `rust-embed`. No Node.js runtime at deployment.
- **REQ-005:** Every new MCP tool introduced in v2.2 MUST be defined in `shared-types` (request/response DTOs) and registered in the `triumvirate` binary's MCP bridge. Crates provide the logic; the binary provides the wiring.

---

## Feature 1: Triumvirate Ledger

Replaces the broken stenographer. SQLite-backed session persistence with health visibility.

### Storage

- **REQ-006:** The Ledger MUST use SQLite in WAL (Write-Ahead Logging) mode. No JSON state files. No mutable coordination flags.
- **REQ-007:** The Ledger database MUST be per-project at `<project>/.triumvirate/ledger.db`. Projects MUST NOT share mutable state. An optional global read-only index at `~/.triumvirate/ledger-index.db` MAY aggregate across projects for cross-project search.
- **REQ-007a:** Project root resolution algorithm (in priority order): (1) `TRIUMVIRATE_PROJECT_ROOT` env var if set (absolute path required), (2) `git rev-parse --show-toplevel` from hook CWD, (3) nearest ancestor directory containing `.triumvirate/`, (4) `~/.triumvirate/scratch/<sha256(cwd)>` as last resort. Raw `$HOME` MUST NOT be used as a project root.
- **REQ-008:** The Ledger MUST define these tables:
  - `events` — append-only raw hook payloads (session_id, event_type, sequence, timestamp, payload_json). Idempotency key: `session_id + event_type + sequence`.
  - `summaries` — compressed observations (event_id FK, title, narrative, facts_json, concepts_json, affected_files_json, summary_type).
  - `sessions` — session metadata (session_id, project, branch, started_at, ended_at, event_count, summary_count).
  - `health` — heartbeat records (timestamp, last_event_id, db_size_bytes, queue_depth).
- **REQ-009:** The Ledger MUST enable FTS5 on the `summaries` table for full-text search over title, narrative, and facts.

### Ingestion

- **REQ-010:** Claude Code hooks (SessionStart, PostToolUse, Stop, SessionEnd) MUST send raw events to the daemon via HTTP POST to `POST /ledger/ingest` with a 200ms timeout. The hook's ONLY job is: validate payload, POST to daemon, return. No summarization in the hook path. The daemon is the single SQLite writer — hooks MUST NOT open direct SQLite connections.
- **REQ-010a:** Hooks MUST NOT use `sqlite3` CLI or any direct database access. The daemon owns all write connections to `ledger.db`. If the daemon HTTP POST fails (timeout or connection refused), the hook falls back to spool (REQ-011).
- **REQ-011:** If the SQLite write fails, the hook MUST append the event as NDJSON to a spool file at `<project>/.triumvirate/ledger-spool.ndjson`.
- **REQ-011a:** The daemon MUST drain the spool into SQLite on startup and every 30 seconds while the spool is non-empty. Spool entries MUST be replayed in strict append order. Successfully replayed entries MUST be removed from the spool file.
- **REQ-011b:** The spool file MUST NOT exceed 100MB. When the limit is reached, the spool MUST rotate to `.ndjson.1` / `.ndjson.2` and the Ledger health endpoint MUST return status `degraded`.
- **REQ-011c:** Spool replay MUST use idempotency keys (`session_id + event_type + sequence`) to deduplicate events that may have been partially written to both spool and DB.
- **REQ-012:** An async compression worker MUST consume raw events and produce summaries using Tier 0: local extractive summary — deterministic, zero API cost. This is the default and always-on path. The worker MUST run as a background Tokio task inside the daemon process, sharing the daemon's SQLite connection pool.
- **REQ-012b:** The daemon MUST run one compression worker pool per project DB. Concurrency MUST be configurable via `TRIUMVIRATE_LEDGER_WORKER_CONCURRENCY` (default: 1).
- **REQ-012a:** Tier 1 LLM-powered abstraction MUST activate only when a session window contains 3+ error events, 5+ file edits, or a user-tagged `ledger_record` call. Tier 1 is capped at `TRIUMVIRATE_LEDGER_LLM_MAX_CALLS_PER_DAY` (default: 20). When the cap is reached, Tier 0 handles all remaining compression for the day.
- **REQ-013:** The compression worker MUST be decoupled from ingestion. Worker failure MUST NOT block or lose raw event capture.
- **REQ-013a:** Worker state MUST use a per-job state machine in the `events` table: `pending → running → done | failed`.
- **REQ-013b:** Running jobs MUST update a heartbeat timestamp at least every 30 seconds. Jobs with heartbeats older than 90 seconds MUST auto-reset to `pending`.

### Retrieval

- **REQ-014:** The Ledger MUST expose MCP tools accessible to ALL three agents (Claude, Gemini, Codex):
  - `ledger_query(query: string, project?: string, limit?: int)` — FTS5 search over summaries. Returns ranked results with session context.
  - `ledger_session(session_id: string)` — Full session reconstruction: metadata + events + summaries.
  - `ledger_record(title: string, narrative: string, facts: string[], concepts: string[])` — Manual high-signal recording by any agent. Writes directly to `summaries`.
  - `ledger_health()` — Returns last event timestamp, queue depth, spool size, DB size, stale job count.

### Health

- **REQ-015:** The daemon MUST expose a `GET /ledger/health` HTTP endpoint returning: last event timestamp, events in last 5 minutes, compression queue depth, spool file size, database size, stale running jobs count.
- **REQ-016:** If no events have been written in the last 5 minutes during an active session, the health endpoint MUST return status `degraded`. The dashboard MUST display this prominently.
- **REQ-017-L:** The `triumvirate doctor` CLI subcommand MUST include Ledger diagnostics: DB exists, WAL mode enabled, spool empty, no stale jobs, last event recency.

### Migration

- **REQ-018:** The existing `outbox.jsonl` and `memory.jsonl` MUST continue to function during migration. The Ledger runs alongside, not instead of, until v2.3 removes the JSONL paths. The `ledger` crate MUST NOT modify or depend on daemon-core's JSONL files.

---

## Feature 2: Lessons Ledger (FEAT-031)

Machine-readable lessons with confidence decay, stored in the Ledger's SQLite.

- **REQ-019:** The `ledger` crate MUST define a `lessons` table: lesson_id, title, body, source_session_id, created_at, last_validated_at, confidence (float 0.0–1.0), tags_json, req_ids_json.
- **REQ-020-LL:** Lessons MUST decay in confidence over time. The decay function: `confidence = initial_confidence * e^(-lambda * days_since_last_validation)`. Default lambda = 0.01 (half-life ~69 days). Lessons below confidence 0.1 are marked `stale`.
- **REQ-021:** MCP tools for lessons:
  - `lesson_add(title, body, tags, confidence?)` — Creates a lesson. Default confidence = 0.8.
  - `lesson_query(query, min_confidence?)` — FTS5 search filtered by confidence threshold.
  - `lesson_validate(lesson_id)` — Resets `last_validated_at` to now, restoring confidence.
  - `lesson_list(tags?, stale?)` — List lessons, optionally filtered.
- **REQ-022:** When the Ledger's compression worker produces a summary with `summary_type` of `error_resolution`, `bug_fix`, or `architecture_decision`, it MUST auto-create a lesson via `lesson_add` with confidence 0.6 (lower than the 0.8 default for manual lessons).

---

## Feature 3: Cross-Model Peer Review

Agents cannot approve their own work. Every approval requires a different model.

- **REQ-023:** The `peer-review` crate MUST define a `ReviewRequest` struct: review_id, author_agent (claude|gemini|codex), artifact (diff, file path, or inline content), review_type (code|architecture|decision), requested_at.
- **REQ-024-PR:** A review MUST be assigned to an agent that is NOT the author. Assignment strategy: round-robin among available non-author agents. If only one other agent is available, it gets the review.
- **REQ-025-PR:** Review results MUST be stored in the Ledger's SQLite: review_id, reviewer_agent, verdict (approve|request_changes|reject), comments, reviewed_at.
- **REQ-026:** MCP tools for peer review:
  - `review_request(artifact, review_type, author_agent)` — Creates a review request and assigns a reviewer.
  - `review_submit(review_id, verdict, comments)` — Submits review from the assigned reviewer.
  - `review_status(review_id?)` — Status of pending/completed reviews.
- **REQ-027:** Peer review is OPTIONAL for standalone agent work by default.
- **REQ-027a:** Peer review MUST be MANDATORY for fleet merge operations (REQ-040).
- **REQ-027b:** Setting `TRIUMVIRATE_REQUIRE_PEER_REVIEW=1` MUST make peer review mandatory for ALL agent output, not just fleet merges.

---

## Feature 4: Fleet/Swarm Execution

Parallel agent execution with worktrees, task lists, and sequential merge.

### Worktree Management

- **REQ-028:** The `fleet` crate MUST manage git worktrees for parallel agent execution. Each fleet member gets an isolated worktree branched from the current HEAD.
- **REQ-029:** Worktree lifecycle: create (branch from HEAD) → agent works → merge back → cleanup. The `GitOps` trait (REQ-003) abstracts all git operations.
- **REQ-030:** Worktree creation MUST verify the source branch is clean (`git diff --quiet`). If uncommitted changes exist, fleet spawn MUST fail with an actionable error, not silently proceed.

### Task Management

- **REQ-031:** The `fleet` crate MUST define a task list stored in the Ledger's SQLite: task_id, fleet_id, title, description, assigned_agent, state (pending|claimed|in_progress|done|failed|blocked), depends_on (list of task_ids), created_at, completed_at.
- **REQ-032:** Task claiming MUST be atomic — two agents cannot claim the same task. SQLite row-level locking handles this.
- **REQ-033:** Tasks with unmet `depends_on` MUST NOT be claimable. The fleet engine resolves the dependency graph and exposes only unblocked tasks.

### Fleet Orchestration

- **REQ-034:** A fleet is defined by: fleet_id, task (description), agent_composition (map of agent_type → count), created_at, state (spawning|running|merging|done|failed).
- **REQ-035:** MCP tools for fleet:
  - `fleet_spawn(task: string, agents: {claude?: int, gemini?: int, codex?: int})` — Creates fleet, worktrees, and task list. Returns fleet_id.
  - `fleet_status(fleet_id)` — Agent states, task progress, worktree status.
  - `fleet_task_list(fleet_id)` — All tasks with state and assignment.
  - `fleet_cancel(fleet_id)` — Kills agents, cleans up worktrees.
- **REQ-036:** The fleet engine MUST emit progress events to the Ledger: fleet spawned, agent started, task claimed, task completed, merge started, merge result, fleet done.
- **REQ-037:** Each fleet member agent MUST receive: the task description, the task list (their assigned tasks), and the worktree path. They do NOT receive accumulated conversation history from other agents.

### Merge

- **REQ-038:** Fleet merge MUST be sequential — one worktree merged at a time, in task completion order. This prevents merge conflicts from compounding.
- **REQ-039:** If a merge produces conflicts, the fleet engine MUST log the conflict details (files, conflict markers, authoring agent) to the Ledger.
- **REQ-039a:** The fleet engine MUST surface merge conflicts to the user via MCP notification within 1 second of detection. The notification MUST name the conflicting files and the agents involved.
- **REQ-039b:** The merge queue MUST pause on conflict. No subsequent worktrees merge until the conflict is resolved. The fleet engine MUST NOT auto-resolve conflicts.
- **REQ-040:** Before merging any worktree, the fleet engine MUST request peer review (REQ-023) of the worktree's diff from an agent that did NOT author the changes.
- **REQ-040a:** Merge MUST NOT proceed until the review verdict is `approve`. A `request_changes` or `reject` verdict MUST pause the merge and surface the reviewer's comments to the user.
- **REQ-040b:** Setting `TRIUMVIRATE_FLEET_SKIP_REVIEW=1` MUST bypass the peer review gate for fleet merges. Skipped reviews MUST be logged to the Ledger with `summary_type = "review_skipped"`.

---

## Feature 5: Dashboard

Svelte 5 + Tailwind 4 web UI embedded in the daemon binary.

### Architecture

- **REQ-041:** The dashboard MUST be a Svelte 5 SPA built with Vite, producing static HTML/CSS/JS.
- **REQ-042:** Static assets MUST be embedded in the `triumvirate` binary via `rust-embed`. The daemon serves them at `GET /` and `GET /assets/*`.
- **REQ-043:** The dashboard MUST connect to the daemon's existing REST API and a new WebSocket endpoint for real-time updates.
- **REQ-044:** The daemon MUST expose `GET /ws` (WebSocket) that streams: agent working state events, fleet progress, Ledger health heartbeats, outbox events.

### Views

- **REQ-045:** The dashboard MUST have a **Sessions** view: active and recent sessions, agent states (idle/working/stuck), live working-state streaming per agent.
- **REQ-045a:** The dashboard MUST have a **Fleet** view: active fleets, task board (kanban-style columns: pending/claimed/in_progress/done/failed), worktree status, merge queue position.
- **REQ-045b:** The dashboard MUST have a **Ledger** view: session history list, FTS5 search input, compression queue depth, health status indicator.
- **REQ-045c:** The dashboard MUST have a **Lessons** view: lesson list with confidence bars (0.0–1.0), stale lessons (confidence < 0.1) highlighted, add/validate action buttons.
- **REQ-045d:** The dashboard MUST have a **Reviews** view: pending reviews with age, review history, approval rate per agent.
- **REQ-045e:** The dashboard MUST have a **Metrics** view: per-agent token usage, cost per session, latency histograms. Data sourced from Prometheus `/metrics` endpoint.
- **REQ-046:** The Sessions view MUST show agent verbosity-filtered working state. The verbosity level (quiet/standard/detailed/raw) MUST be selectable per-agent in the UI, using the `AgentVerbosity` enum from `agent-adapter`.
- **REQ-047:** The Ledger health view MUST prominently display a status indicator: green (healthy — events in last 5 min), yellow (degraded — no events but no active session), red (dead — active session + no events in 5 min). This is the "never silently die" requirement.

### Design System

- **REQ-048:** The dashboard MUST follow a design system document (`DESIGN_SYSTEM.md`) specifying: color palette (hex values), typography (font families, sizes, weights), spacing scale, border radius, shadows, breakpoints, and component variants. No ad-hoc styling.

---

## Feature 6: OutboxEvent Enrichment (REQ-017 from v2.1)

- **REQ-049:** The `OutboxEvent` struct in `shared-types` MUST be extended with `working_state: Option<String>` — backward-compatible, old readers ignore it.
- **REQ-049a:** The `OutboxEvent` struct MUST be extended with `token_usage: Option<TokenUsage>` — populated from `ParsedAgentResult`.
- **REQ-049b:** The `OutboxEvent` struct MUST be extended with `tool_name: Option<String>` — the last tool call name from agent output.
- **REQ-050:** The agent execution path in `triumvirate/src/agent_exec.rs` MUST populate these fields from the `ParsedAgentResult` returned by `agent-adapter` parsers.

---

## Feature 7: Codex App-Server Handshake (REQ-020 from v2.1)

- **REQ-051:** The `agent-adapter` crate MUST implement a `CodexAppServerParser` that handles the JSON-RPC 2.0 protocol: `initialize → initialized → thread/start → turn/start → stream notifications → turn/completed`.
- **REQ-052:** The parser MUST handle Codex approval requests (`approval_request` events). When received, the daemon MUST auto-approve if `TRIUMVIRATE_CODEX_AUTO_APPROVE=1` (default), or surface to the user via MCP notification if disabled.
- **REQ-053:** The `TRIUMVIRATE_CODEX_PROTOCOL` env var MUST support values `exec` (current default) and `app-server`. The daemon selects the parser based on this value.

---

## Feature 8: App-Server Auto-Approve (REQ-024 from v2.1)

- **REQ-054:** When `TRIUMVIRATE_CODEX_AUTO_APPROVE=1`, the daemon MUST respond to Codex approval requests with `ProceedOnce` within 100ms. This enables unattended Codex execution in fleet scenarios.
- **REQ-055:** Auto-approved actions MUST be logged to the Ledger with `summary_type = "auto_approved"` for audit trail.

---

## Feature 9: Outbox Rotation/GC (REQ-025 from v2.1)

- **REQ-056:** Outbox events written to the Ledger's `events` table MUST have a retention policy: events older than 30 days with no associated summary are eligible for deletion. Events WITH summaries are retained indefinitely (the summary is the compressed record).
- **REQ-057:** A `ledger_gc()` MCP tool MUST trigger garbage collection. It MUST return: events scanned, events deleted, space reclaimed. GC MUST also clear acknowledged dead-drop tickets older than 7 days.
- **REQ-058:** The daemon MUST run GC automatically on startup if last GC was >24 hours ago. GC MUST NOT run during active fleet operations (check `fleets` table for state != `done` and state != `failed`).

---

## Non-Goals for v2.2

- **NG-001:** Vector search / embeddings. FTS5 is sufficient. Revisit if recall is measurably bad.
- **NG-002:** Remote agents (GCE, RunPod). Fleet is local-only for now.
- **NG-003:** Multi-user support. Single operator.
- **NG-004:** Plugin system for new agent types. Gemini + Codex only.
- **NG-005:** Cedar governance / policy engine. Peer review is the governance layer for v2.2.
- **NG-006:** API-mode (direct Anthropic/OpenAI/Google API). CLI-first stays.

---

## Success Criteria

1. A session on the Triumvirate project produces Ledger entries visible in the dashboard within 5 seconds of each tool call.
2. `triumvirate doctor` reports Ledger health as green during active work.
3. Kill the daemon mid-session, restart it, and verify: zero events lost (spool replay), session resumable.
4. Spawn a 3-agent fleet (1 Claude, 1 Gemini, 1 Codex) on a real task. All three get worktrees, claim tasks, produce output. Merge succeeds with peer review gate.
5. A lesson added today shows confidence < 0.5 after 50 days without validation.
6. An agent attempting to approve its own fleet merge is rejected with an error naming the required reviewer.
7. The dashboard loads at `localhost:8080`, shows live agent state, and the Ledger health indicator is green.
