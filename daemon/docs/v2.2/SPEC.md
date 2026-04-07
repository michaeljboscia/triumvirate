# Triumvirate v2.2 — The Accountability Release

**Author:** Claude (orchestrator), with Gemini + Codex twin review
**Date:** 2026-04-07
**Branch:** TBD (feat/v2.2-accountability)
**Status:** DRAFT — goat rodeo Round 7 complete, pending Decision Ledger

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
- **REQ-009a:** The daemon's SQLite write coordinator MUST prioritize event ingestion writes over task-state updates. Under burst load (fleet agents claiming tasks while hooks write events), ingestion MUST NOT be starved. The daemon MUST expose a `ledger_queue_lag` Prometheus metric and emit `degraded` health status if ingestion lag exceeds 5 seconds.

### Ingestion

- **REQ-010:** Claude Code hooks (SessionStart, PostToolUse, Stop, SessionEnd) MUST write raw events to the project's spool directory (`<project>/.triumvirate/spool/`) as their PRIMARY action. The hook writes the event JSON to a temp file (`event-<ts>-<pid>-<rand>.tmp`), then atomically renames it to `event-<ts>-<pid>-<rand>.ndjson`. Filename generation MUST use POSIX-safe builtins only: `event-$(date +%s)-$$-$RANDOM.tmp`. No dependency on `uuidgen` or external binaries. This eliminates all file contention — no `flock`, no interleaving, no corruption from concurrent hooks. After writing, the hook MUST fire a non-blocking background HTTP ping to the daemon (`POST /ledger/wake` with JSON body `{"project_root": "<absolute path>"}`, no response wait). The hook's ONLY job is: validate payload, atomic spool write, ping daemon, return.
- **REQ-010b:** The daemon MUST maintain an in-memory LRU cache of recently active project roots (populated by `/ledger/wake` calls). The daemon MUST sweep known project spool directories every 60 seconds as a safety net. Spool files MUST be processed in creation-time order and deleted after successful ingestion into SQLite.
- **REQ-010c:** Hooks MUST write the full event payload to the spool file with no size cap — bash MUST NOT truncate JSON. The daemon's ingestion pipeline MUST handle oversized payloads: fields exceeding 64KB (e.g., tool stdout) MUST be truncated by the daemon with a `"[...truncated]"` marker while preserving valid JSON structure. Truncation is the daemon's responsibility, not the hook's.
- **REQ-010a:** Hooks MUST NOT use `sqlite3` CLI, direct database access, or synchronous HTTP calls. The spool directory is the durable ingestion path. The daemon is the single SQLite writer — it drains the spool directory asynchronously. The hook cannot fail unless the local filesystem is unavailable.
- **REQ-011:** The spool directory (`<project>/.triumvirate/spool/`) is the PRIMARY ingestion path (REQ-010). This REQ covers spool OVERFLOW: if the spool directory exceeds 100MB total size, new events MUST still be written (spool is never rejected) but the Ledger health MUST report `degraded`.
- **REQ-011a:** The daemon MUST drain the spool directory into SQLite on startup, on each `/ledger/wake` ping, and every 60 seconds (REQ-010b). Spool files MUST be processed in creation-time order. Successfully ingested files MUST be deleted.
- **REQ-011b:** Spool replay MUST use idempotency keys (`session_id + event_type + sequence`) to deduplicate events that may have been processed from a previous partial drain.
- **REQ-012:** An async compression worker MUST consume raw events and produce summaries using Tier 0: local extractive summary — deterministic, zero API cost. This is the default and always-on path. The worker MUST run as a background Tokio task inside the daemon process, sharing the daemon's SQLite connection pool.
- **REQ-012b:** The daemon MUST run one compression worker pool per project DB, lazily initialized on the first event for that project. Worker pools MUST shut down after 15 minutes of zero events (idle TTL). Maximum active pools MUST be capped at `TRIUMVIRATE_LEDGER_MAX_POOLS` (default: 10) to prevent descriptor/thread creep. Concurrency per pool MUST be configurable via `TRIUMVIRATE_LEDGER_WORKER_CONCURRENCY` (default: 1).
- **REQ-012a:** Tier 1 LLM-powered abstraction MUST activate only when a session window contains 3+ error events, 5+ file edits, or a user-tagged `ledger_record` call. Tier 1 is capped at `TRIUMVIRATE_LEDGER_LLM_MAX_CALLS_PER_DAY` (default: 20). When the cap is reached, Tier 0 handles all remaining compression for the day.
- **REQ-013:** The compression worker MUST be decoupled from ingestion. Worker failure MUST NOT block or lose raw event capture.
- **REQ-013a:** Worker state MUST use a per-job state machine in the `events` table: `pending → running → done | failed`.
- **REQ-013b:** Running jobs MUST update a heartbeat timestamp at least every 30 seconds. Jobs with heartbeats older than 90 seconds MUST auto-reset to `pending`.

### Retrieval

- **REQ-014:** The Ledger MUST expose MCP tools accessible to Claude directly via MCP protocol:
  - `ledger_query(query: string, project?: string, limit?: int)` — FTS5 search over summaries. Returns ranked results with session context.
  - `ledger_session(session_id: string)` — Full session reconstruction: metadata + events + summaries.
  - `ledger_record(title: string, narrative: string, facts: string[], concepts: string[])` — Manual high-signal recording by any agent. Writes directly to `summaries`.
  - `ledger_health()` — (Moved to REQ-015a in Health section — ships in Phase 1).
- **REQ-014a:** Gemini and Codex (text-in/text-out subprocesses) MAY access ledger tools via structured XML markers in their stdout (e.g., `<triumvirate_tool name="ledger_record">...</triumvirate_tool>`). This is an OPPORTUNISTIC high-signal capture path, not the primary durability mechanism. The `agent-adapter` crate MUST parse these markers when present, execute the tool in the daemon, and inject the result back into the agent's context. If the agent outputs malformed XML, the adapter MUST return a synthetic error message to trigger self-correction.
- **REQ-014b:** The daemon MUST inject XML tool usage instructions into the startup prompt for Gemini and Codex agents. Core session capture (REQ-010) remains the primary durability path — XML markers are additive, not required. The daemon MUST track `marker_parse_success_rate` as a Prometheus metric and emit a `degraded` health warning if the rate falls below 50% over a 1-hour window.

### Health

- **REQ-015:** The daemon MUST expose a `GET /ledger/health` HTTP endpoint returning: last event timestamp, events in last 5 minutes, compression queue depth, spool directory size, database size, stale running jobs count.
- **REQ-015a:** The Ledger MUST expose a `ledger_health()` MCP tool returning the same data as the HTTP endpoint (REQ-015). This ships in Phase 1 alongside the HTTP endpoint — it is NOT gated behind Phase 2 retrieval tools.
- **REQ-016:** If no events have been written in the last 5 minutes during an active session, the health endpoint MUST return status `degraded`. The dashboard MUST display this prominently.
- **REQ-017-L:** The `triumvirate doctor` CLI subcommand MUST include Ledger diagnostics: DB exists, WAL mode enabled, spool empty, no stale jobs, last event recency.

### Migration

- **REQ-018:** The existing `outbox.jsonl` and `memory.jsonl` MUST continue to function during migration. The Ledger runs alongside, not instead of, until v2.3 removes the JSONL paths. The `ledger` crate MUST NOT modify or depend on daemon-core's JSONL files.
- **REQ-018a:** Ledger Phase 1 initialization MUST ensure `.triumvirate/` is listed in the project's root `.gitignore`. If not present, the Ledger MUST append it. This prevents `ledger.db`, spool directory, and runtime artifacts from polluting `git status`. This is a Ledger responsibility, not a fleet responsibility — it ships in Phase 1 before fleet exists.

---

## Feature 2: Lessons Ledger (FEAT-031)

Machine-readable lessons with confidence decay, stored in the Ledger's SQLite.

- **REQ-019:** The `ledger` crate MUST define a `lessons` table: lesson_id, title, body, source_session_id, created_at, last_validated_at, confidence (float 0.0–1.0), tags_json, req_ids_json.
- **REQ-020-LL:** Lessons MUST decay in confidence over time. The decay function: `confidence = initial_confidence * e^(-lambda * days_since_last_validation)`. Default lambda = 0.01 (half-life ~69 days). Lessons below confidence 0.1 are marked `stale`. Decay MUST be calculated at query time (not background mutation). The dashboard MAY cache a daily materialized snapshot for display performance.
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
- **REQ-024-PR:** A review MUST be assigned to a full agent that is NOT the author. Assignment strategy: round-robin among registered non-author agent types (claude, gemini, codex). The daemon MUST spawn a reviewer on demand via `ask_agent` if one is not already running. Reviews use full agent sessions, not lightweight models.
- **REQ-024a:** The `peer-review` crate MUST implement a bounded review queue. Maximum concurrent in-flight reviews MUST be capped at `TRIUMVIRATE_REVIEW_MAX_INFLIGHT` (default: 2). Reviews exceeding the cap are queued FIFO. Review jobs MUST have a 120-second timeout — on timeout, the review is marked `failed` and the merge queue surfaces the failure to the user.
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
- **REQ-032:** Task claiming MUST be atomic — two agents cannot claim the same task. The daemon owns the single write connection to the source project's `ledger.db`. Agents claim tasks by calling `fleet_claim_task` MCP tool, which the daemon executes as a single SQLite transaction (`UPDATE tasks SET state='claimed', assigned_agent=? WHERE task_id=? AND state='pending'` + check `changes() == 1`).
- **REQ-033:** Tasks with unmet `depends_on` MUST NOT be claimable. The fleet engine resolves the dependency graph and exposes only unblocked tasks.

### Fleet Orchestration

- **REQ-034:** A fleet is defined by: fleet_id, task (description), agent_composition (map of agent_type → count), created_at, state (spawning|running|merging|done|failed).
- **REQ-035:** MCP tools for fleet:
  - `fleet_spawn(task: string, agents: {claude?: int, gemini?: int, codex?: int}, dry_run?: bool, wait?: bool)` — When `dry_run=true` (or omitted), returns a plan summary: agent count, worktree locations, estimated tasks, current HEAD. Claude presents this to the user for confirmation. When `dry_run=false`, executes the plan: creates worktrees, spawns agents, returns fleet_id with `state=spawning`. If `wait=true`, blocks until all worktrees are created and agents are running. Streams progress via WebSocket (REQ-044) regardless of wait mode.
  - `fleet_status(fleet_id)` — Agent states, task progress, worktree status.
  - `fleet_task_list(fleet_id)` — All tasks with state and assignment.
  - `fleet_cancel(fleet_id)` — Kills agents, cleans up worktrees.
- **REQ-036:** The fleet engine (running in the daemon) MUST emit progress events to the source project's Ledger: fleet spawned, agent started, task claimed, task completed, merge started, merge result, fleet done. Fleet members in `/tmp/` worktrees MUST have `TRIUMVIRATE_PROJECT_ROOT` set to the source project root so all events route to the correct `ledger.db`.
- **REQ-037:** Each fleet member agent MUST receive task assignments via two channels:
  - A machine-readable file at `.triumvirate/fleet-task.md` in the worktree, containing frontmatter (task_id, fleet_id, assigned_agent, depends_on) and prose description. This file is UNTRACKED runtime metadata — invisible to `git status` because `.triumvirate/` is in `.gitignore` (owned by Ledger Phase 1 initialization, REQ-018a). The task file is immune to merge conflicts.
  - A startup prompt summarizing the task and referencing the file: "Your task assignment is at .triumvirate/fleet-task.md — read it before starting."
  Fleet members MUST NOT receive accumulated conversation history from other agents.

### Merge

- **REQ-038:** Fleet merge MUST be sequential — one worktree merged at a time, in task completion order. This prevents merge conflicts from compounding.
- **REQ-039:** If a merge produces conflicts, the fleet engine MUST log the conflict details (files, conflict markers, authoring agent) to the Ledger.
- **REQ-039a:** The fleet engine MUST surface merge conflicts to the user via MCP notification within 1 second of detection. The notification MUST name the conflicting files and the agents involved.
- **REQ-039b:** The merge queue MUST pause on conflict. No subsequent worktrees merge until the conflict is resolved. The fleet engine MUST NOT auto-resolve conflicts.
- **REQ-040:** The fleet engine MUST request peer review (REQ-023) of each worktree's diff as soon as the authoring agent completes its task — NOT at merge time. Reviews run in parallel with other agents still working. By the time the merge phase begins, most reviews are already complete.
- **REQ-040a:** The merge phase (REQ-038) MUST check review status before each sequential merge. Merge MUST NOT proceed for a worktree until its review verdict is `approve`. A `request_changes` or `reject` verdict MUST pause the merge queue and surface the reviewer's comments to the user.
- **REQ-040b:** Setting `TRIUMVIRATE_FLEET_SKIP_REVIEW=1` MUST bypass the peer review gate for fleet merges. Skipped reviews MUST be logged to the Ledger with `summary_type = "review_skipped"`.

---

## Feature 5: Dashboard

Svelte 5 + Tailwind 4 web UI embedded in the daemon binary.

### Architecture

- **REQ-041:** The dashboard MUST be a Svelte 5 SPA built with Vite, producing static HTML/CSS/JS.
- **REQ-042:** Static assets MUST be embedded in the `triumvirate` binary via `rust-embed`. The daemon serves them at `GET /` and `GET /assets/*`.
- **REQ-043:** The dashboard MUST connect to the daemon's existing REST API and a new WebSocket endpoint for real-time updates.
- **REQ-044:** The daemon MUST expose `GET /ws` (WebSocket) that streams: agent working state events, fleet progress, Ledger health heartbeats, outbox events.
- **REQ-044a:** The MCP stdio bridge (`triumvirate mcp`) MUST operate exclusively as a proxy to the daemon. Local agent execution (without the daemon) MUST be removed. If the daemon is unreachable and auto-start fails, the MCP bridge MUST return an explicit error including recovery instructions (`triumvirate daemon` to start manually). The daemon MUST implement auto-restart with exponential backoff on crash.

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

- **REQ-048:** The dashboard MUST follow a design system document at `dashboard/DESIGN_SYSTEM.md` specifying: color palette (hex values), typography (font families, sizes, weights), spacing scale, border radius, shadows, breakpoints, and component variants. No ad-hoc styling. This document is a spec deliverable — it MUST be written and approved before dashboard development begins. It is source-controlled and dashboard PRs MUST reference updated design tokens when applicable.

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

- **REQ-054:** When `TRIUMVIRATE_CODEX_AUTO_APPROVE=1`, the daemon MUST append `--full-auto` (or the current equivalent flag) to the Codex CLI command when spawning the subprocess. This enables unattended Codex execution in fleet scenarios without depending on the app-server JSON-RPC approval response channel (which is broken as of early 2026).
- **REQ-054a:** On Codex subprocess startup, the daemon MUST probe whether the app-server approval response channel is functional. If functional, the daemon MAY use JSON-RPC `ProceedOnce` responses instead of `--full-auto`. If non-functional, `--full-auto` is the mandatory fallback.
- **REQ-055:** Auto-approved actions MUST be logged to the Ledger with `summary_type = "auto_approved"` for audit trail.

---

## Feature 9: Outbox Rotation/GC (REQ-025 from v2.1)

- **REQ-056:** Outbox events written to the Ledger's `events` table MUST have a retention policy: events older than 30 days with no associated summary are eligible for deletion. Events WITH summaries are retained indefinitely (the summary is the compressed record).
- **REQ-057:** A `ledger_gc()` MCP tool MUST trigger garbage collection. It MUST return: events scanned, events deleted, space reclaimed. GC MUST also clear acknowledged dead-drop tickets older than 7 days.
- **REQ-058:** The daemon MUST run GC automatically on startup if last GC was >24 hours ago. GC MUST NOT run during active fleet operations (check `fleets` table for state != `done` and state != `failed`).

---

## Phased Implementation Order

v2.2 ships incrementally. Each phase is independently shippable and testable. Later phases depend on earlier ones but not vice versa.

| Phase | Crate(s) | REQs | Ships | Rollback? |
|-------|----------|------|-------|-----------|
| **1 — Data** | `shared-types` (DTOs), `ledger` (ingestion/spool/drain/health) | REQ-006–018 | Fixes stenographer. Ledger captures events. | Yes — JSONL still works (REQ-018) |
| **2 — Knowledge** | `ledger` (retrieval + lessons) | REQ-014–014b, REQ-019–022 | FTS5 search, lesson CRUD, confidence decay | Yes — Phase 1 still captures without retrieval |
| **3 — Fleet Core** | `fleet` (worktrees/tasks/claiming/progress) | REQ-028–037 | Parallel agents with task lists. No review gate yet. | Yes — behind `TRIUMVIRATE_FLEET_ENABLED` flag |
| **4 — Review** | `peer-review`, fleet merge gating | REQ-023–027b, REQ-040–040b | Cross-model review. Fleet merge gate. | Yes — `TRIUMVIRATE_FLEET_SKIP_REVIEW=1` disables |
| **5 — Visibility** | `dashboard` (Svelte + rust-embed + WebSocket) | REQ-041–048 | Dashboard with all views. | Yes — daemon works without dashboard |
| **6 — Codex Protocol** | `agent-adapter` (app-server parser + auto-approve) | REQ-049–055 | Codex app-server support. | Yes — falls back to exec protocol |
| **7 — Polish** | GC/retention | REQ-056–058 | Outbox rotation, automatic cleanup. | Yes — no GC = unbounded growth but not broken |

**Dependency chain:** Phase 1 → Phase 2 → (Phase 3 and Phase 5 can run in parallel) → Phase 4 requires Phase 3 → Phase 6 and 7 are independent of 3-5.

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

1. A session on the Triumvirate project produces Ledger entries visible in the dashboard with p95 event-to-dashboard latency under 2 seconds. Measured as: hook spool write → daemon drain → SQLite insert → WebSocket broadcast → dashboard render.
2. `triumvirate doctor` reports Ledger health as green during active work.
3. Kill the daemon mid-session, restart it, and verify: zero events lost (spool replay), session resumable.
4. Spawn a 3-agent fleet (1 Claude, 1 Gemini, 1 Codex) on a real task. All three get worktrees, claim tasks, produce output. Merge succeeds with peer review gate.
5. A lesson added today shows confidence < 0.5 after 50 days without validation.
6. An agent attempting to approve its own fleet merge is rejected with an error naming the required reviewer.
7. The dashboard loads at `localhost:8080`, shows live agent state, and the Ledger health indicator is green.
