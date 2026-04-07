# Implementation Plan — Triumvirate v2.2

---

## Phase 1: Data (Ledger Ingestion + Health)

**Crates:** `shared-types` (new DTOs + GitOps trait), `ledger` (new crate)
**REQs:** 001–005, 006–013b, 015–018a

### Wave 0 — Contracts

- [ ] **1.0** Add `GitOps` trait to `shared-types`.

<task id="T-100" req="REQ-003" wave="0" depends="">
  <description>Define GitOps trait in shared-types with worktree_add, worktree_remove, is_clean, current_head, merge, diff, rev_parse_toplevel</description>
  <files>shared-types/src/git_ops.rs</files>
  <contract>pub trait GitOps: Send + Sync { async fn worktree_add, worktree_remove, is_clean, current_head, merge, diff, rev_parse_toplevel }</contract>
  <verify>cargo check -p shared-types</verify>
  <reality_test>Import GitOps in a test module. Create a mock impl. Call each method. Verify the trait compiles with async_trait and the mock returns expected types. A stub that only defines the trait without method signatures fails.</reality_test>
</task>

- [ ] **1.1** Add Ledger DTOs to `shared-types`.

<task id="T-101" req="REQ-005" wave="0" depends="">
  <description>Define RawEvent, Summary, HealthStatus, DrainResult, NewLesson, GcResult, ManualRecord, SessionDetail DTOs with Serialize/Deserialize/JsonSchema</description>
  <files>shared-types/src/ledger.rs</files>
  <contract>pub struct RawEvent { session_id, event_type, sequence, timestamp, payload_json }; pub struct Summary { title, narrative, facts_json, ... }; etc.</contract>
  <verify>cargo check -p shared-types</verify>
  <reality_test>Round-trip every DTO through serde_json: construct → serialize → deserialize → assert fields match. A struct with wrong field types or missing Serialize/Deserialize fails.</reality_test>
</task>

- [ ] **1.2** Add `LedgerStore` public API signatures to `ledger/src/lib.rs`.

<task id="T-102" req="REQ-002a" wave="0" depends="">
  <description>Define LedgerStore struct with public method signatures: open(PathBuf), ingest_event, drain_spool, query, get_session, record, health, add_lesson, query_lessons, validate_lesson, gc. All take absolute PathBuf. No git resolution.</description>
  <files>ledger/src/lib.rs</files>
  <contract>pub struct LedgerStore; impl LedgerStore { pub fn open(project_root: PathBuf) -> Result&lt;Self&gt;; pub fn ingest_event(&amp;self, event: RawEvent) -> Result&lt;()&gt;; ... }</contract>
  <verify>cargo check -p ledger</verify>
  <reality_test>Call LedgerStore::open with a relative path → must return error (not panic, not silently convert). Call with absolute path → must succeed. A stub that accepts any path fails the relative-path rejection test.</reality_test>
</task>

### Wave 1 — SQLite Foundation

- [ ] **1.3** Create `ledger` crate with `Cargo.toml`.

<task id="T-103" req="REQ-001" wave="1" depends="T-101,T-102">
  <description>Create ledger crate Cargo.toml with rusqlite (bundled feature), serde_json, shared-types, agent-adapter deps</description>
  <files>ledger/Cargo.toml</files>
  <verify>cargo check -p ledger</verify>
  <reality_test>cargo build -p ledger succeeds. Cargo.toml does NOT list daemon-core as dependency (REQ-002). grep for daemon-core in Cargo.toml returns empty.</reality_test>
</task>

- [ ] **1.4** Implement `LedgerStore::open()` — create DB with WAL + PRAGMAs + all tables.

<task id="T-104" req="REQ-006,REQ-008" wave="1" depends="T-102,T-103">
  <description>Implement open() that creates SQLite DB in WAL mode, sets busy_timeout=5000, synchronous=NORMAL, creates events/summaries/sessions/health/lessons tables</description>
  <files>ledger/src/store.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Call open() with a temp dir. PRAGMA journal_mode returns 'wal'. SELECT name FROM sqlite_master returns all 5 tables. A stub that creates an empty DB with no tables fails the table check.</reality_test>
</task>

- [ ] **1.5** Implement FTS5 virtual tables.

<task id="T-105" req="REQ-009" wave="1" depends="T-104">
  <description>Create FTS5 virtual tables summaries_fts and lessons_fts with content sync</description>
  <files>ledger/src/store.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Insert a summary with title "authentication middleware bug". FTS5 MATCH query for "authentication" returns the row. Query for "nonexistent" returns empty. A stub with no FTS5 table fails the MATCH query.</reality_test>
</task>

- [ ] **1.6** Implement `ingest_event()` with idempotency.

<task id="T-106" req="REQ-008" wave="1" depends="T-104">
  <description>Insert event into events table. Idempotency key: session_id + event_type + sequence. Duplicate insert returns Ok without creating duplicate row.</description>
  <files>ledger/src/ingest.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Ingest event A. Ingest event A again (same key). SELECT COUNT(*) WHERE session_id=A returns 1, not 2. A stub that always inserts fails the count check.</reality_test>
</task>

### Wave 2 — Spool + Drain

- [ ] **1.7** Implement `drain_spool()`.

<task id="T-107" req="REQ-010b,REQ-010c,REQ-011a,REQ-011b" wave="2" depends="T-106">
  <description>Read spool dir, sort files by ctime, parse JSON, truncate fields >64KB with [...truncated] marker, ingest via ingest_event, delete processed files. Idempotency dedupe on replay.</description>
  <files>ledger/src/spool.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Write 3 spool files (valid JSON) to temp dir. Call drain_spool. All 3 events in SQLite. Spool dir empty. Write 1 file with 100KB tool_output field. Drain. Field in DB contains "[...truncated]" and is valid JSON. A stub that reads but doesn't delete files fails the empty-dir check.</reality_test>
</task>

- [ ] **1.8** Add `POST /ledger/wake` daemon endpoint.

<task id="T-108" req="REQ-010" wave="2" depends="T-107">
  <description>HTTP POST /ledger/wake accepts {project_root: string}. Updates LRU cache. Triggers drain_spool for that project. Bearer auth required.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Start daemon. Write spool file to project dir. POST /ledger/wake with project_root. Event appears in SQLite within 1s. POST without bearer token returns 401. A stub endpoint that returns 200 without draining fails the SQLite check.</reality_test>
</task>

- [ ] **1.9** Add 60-second spool sweep Tokio task.

<task id="T-109" req="REQ-010b" wave="2" depends="T-107,T-108">
  <description>Background Tokio task sweeps all project roots in LRU cache every 60s. Drains any non-empty spool dirs.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Start daemon. Add project to LRU via /ledger/wake. Write spool file WITHOUT sending another wake ping. Wait 65s. Event appears in SQLite (swept by background task). A stub that never sweeps fails the timed check.</reality_test>
</task>

- [ ] **1.10** Write bash hooks.

<task id="T-110" req="REQ-010,REQ-007a" wave="2" depends="T-108">
  <description>post-tool-use-ledger.sh, session-start-ledger.sh, session-end-ledger.sh. Each: resolve project root (git rev-parse), write event JSON to spool dir via atomic rename (tmp→ndjson), background curl to /ledger/wake. POSIX builtins only (date, $$, $RANDOM).</description>
  <files>~/.claude/hooks/post-tool-use-ledger.sh, ~/.claude/hooks/session-start-ledger.sh, ~/.claude/hooks/session-end-ledger.sh</files>
  <verify>bash -n post-tool-use-ledger.sh</verify>
  <reality_test>Run hook with mock PostToolUse JSON on stdin. Verify: spool dir contains exactly 1 .ndjson file. File is valid JSON (python3 -m json.tool). File does NOT contain sqlite3 or direct DB references (grep). Filename matches pattern event-DIGITS-DIGITS-DIGITS.ndjson. Run 10 hooks in parallel — all 10 files valid, zero corruption.</reality_test>
</task>

### Wave 3 — Health + Doctor

- [ ] **1.11** Implement `health()`.

<task id="T-111" req="REQ-015" wave="3" depends="T-107">
  <description>Query events in last 5 min, spool dir total size, compression queue depth, DB file size, count of stale running jobs. Return HealthStatus.</description>
  <files>ledger/src/health.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Ingest 5 events. Call health(). events_last_5min >= 5. Ingest nothing for 5 min (mock time). Call health(). events_last_5min == 0 AND status == "degraded" if session active. A stub returning hardcoded HealthStatus{healthy} fails the degraded check.</reality_test>
</task>

- [ ] **1.12** Add `GET /ledger/health` HTTP endpoint.

<task id="T-112" req="REQ-015" wave="3" depends="T-111">
  <description>HTTP GET /ledger/health returns JSON with all HealthStatus fields. Bearer auth required.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>curl localhost:8080/ledger/health with valid bearer token. Response is valid JSON containing: last_event_timestamp, events_last_5min, queue_depth, spool_size_bytes, db_size_bytes, stale_jobs, status. Missing any field = fail.</reality_test>
</task>

- [ ] **1.13** Add `ledger_health()` MCP tool.

<task id="T-113" req="REQ-015a" wave="3" depends="T-111">
  <description>Register ledger_health MCP tool in the MCP bridge. Returns same data as HTTP endpoint.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Call ledger_health via MCP protocol (rmcp test client or echo JSON to stdin). Response contains all HealthStatus fields. Compare with HTTP endpoint — same values. A stub returning empty JSON fails field check.</reality_test>
</task>

- [ ] **1.14** Extend `triumvirate doctor` with Ledger diagnostics.

<task id="T-114" req="REQ-017-L" wave="3" depends="T-111">
  <description>Doctor subcommand checks: DB file exists, WAL mode enabled, spool dir empty or draining, no stale jobs, last event recency.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Run triumvirate doctor. Output contains "Ledger:" section with pass/fail for each check. Create a stale job (heartbeat >90s ago). Run doctor again. Output shows FAIL for stale jobs. A stub that always prints "OK" fails the stale-job detection.</reality_test>
</task>

- [ ] **1.15** Implement .gitignore initialization.

<task id="T-115" req="REQ-018a" wave="3" depends="T-104">
  <description>On first LedgerStore::open for a git repo, check if .triumvirate/ is in .gitignore. If not, append it.</description>
  <files>ledger/src/init.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Create temp git repo without .triumvirate/ in .gitignore. Open LedgerStore. Read .gitignore — contains .triumvirate/. Open again — not duplicated. A stub that never touches .gitignore fails the content check.</reality_test>
</task>

### Wave 4 — Compression

- [ ] **1.16** Implement Tier 0 extractive compression worker.

<task id="T-116" req="REQ-012" wave="4" depends="T-106">
  <description>Tokio background task that reads pending events, produces extractive summaries (key facts, affected files, event type), writes to summaries table + FTS5 index. Updates event compression_state to done.</description>
  <files>ledger/src/compression.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Ingest 5 events with different tool calls (Edit, Bash, Read). Wait for worker. summaries table has >=1 row. Summary narrative references the tool types from the events. events.compression_state all 'done'. A stub that marks events done without creating summaries fails the summary content check.</reality_test>
</task>

- [ ] **1.17** Implement job state machine with heartbeat + TTL.

<task id="T-117" req="REQ-013a,REQ-013b" wave="4" depends="T-116">
  <description>Per-job state: pending→running→done|failed. Running jobs update heartbeat every 30s. Jobs with heartbeat >90s auto-reset to pending.</description>
  <files>ledger/src/compression.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Set event to running with heartbeat 2 minutes ago. Run TTL reclaim. Event is now pending. Set event to running with heartbeat 10s ago. Run reclaim. Event still running. A stub that always resets fails the "recent heartbeat stays running" check.</reality_test>
</task>

- [ ] **1.18** Implement lazy worker pool with idle TTL + max cap.

<task id="T-118" req="REQ-012b" wave="4" depends="T-116">
  <description>One pool per project DB. Lazy init on first event. Shutdown after 15 min idle. Max 10 active pools.</description>
  <files>ledger/src/pool.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Create events for 3 projects. Verify 3 pools active. Wait 16 min (mock time). Verify pools shut down. Create events for 11 projects. Verify max 10 pools active, 11th queued. A stub with no pool cap fails the 11-project check.</reality_test>
</task>

- [ ] **1.19** Implement write-path priority + queue lag metric.

<task id="T-119" req="REQ-009a" wave="4" depends="T-107">
  <description>Ingestion writes prioritized over task-state updates. ledger_queue_lag Prometheus metric. Degraded if lag >5s.</description>
  <files>ledger/src/store.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Simulate burst: 50 task updates + 50 ingestion events submitted concurrently. Measure time for all ingestion events to land. Must be <5s. Prometheus metric ledger_queue_lag exists and reports a value. A stub with no priority that processes FIFO may fail under load.</reality_test>
</task>

**Gate:** `triumvirate doctor` reports green. Hook fires, spool drains, event appears in SQLite. Health endpoint returns `healthy`.

---

## Phase 2: Knowledge (Retrieval + Lessons)

**Crates:** `ledger` (extend)
**REQs:** 014–014b, 019–022

- [ ] **2.1** Implement `query()` — FTS5 search over summaries.

<task id="T-201" req="REQ-014" wave="1" depends="T-105">
  <description>FTS5 search over summaries_fts. Returns ranked results with session context.</description>
  <files>ledger/src/query.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Insert 3 summaries: "auth middleware bug", "database migration", "CSS layout fix". Query "auth". Returns only auth summary. Query "migration" returns only migration. Query "nonexistent" returns empty. A stub returning all summaries regardless of query fails.</reality_test>
</task>

- [ ] **2.2** Implement `get_session()` — full session reconstruction.

<task id="T-202" req="REQ-014" wave="1" depends="T-104,T-106">
  <description>Reconstruct full session: metadata + all events + all summaries for a session_id.</description>
  <files>ledger/src/query.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Ingest 10 events for session "abc". Create 2 summaries linked to those events. Call get_session("abc"). Result has 10 events + 2 summaries + correct metadata. Call get_session("nonexistent") returns error. A stub returning empty SessionDetail fails the count check.</reality_test>
</task>

- [ ] **2.3** Implement `record()` — manual high-signal recording.

<task id="T-203" req="REQ-014" wave="1" depends="T-104">
  <description>Direct insert into summaries table from ManualRecord. No event required.</description>
  <files>ledger/src/ingest.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Call record with title="Architecture decision: use WAL". Query FTS5 for "WAL". Returns the record. summary_type is correct. A stub that inserts but doesn't populate FTS5 fails the search.</reality_test>
</task>

- [ ] **2.4** Add `ledger_query`, `ledger_session`, `ledger_record` MCP tools.

<task id="T-204" req="REQ-014" wave="2" depends="T-201,T-202,T-203">
  <description>Register 3 MCP tools in the bridge. Wire to LedgerStore methods.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Call ledger_record via MCP with title="test". Then call ledger_query via MCP with query="test". Returns the record. A stub that registers tools but doesn't wire to store fails the round-trip.</reality_test>
</task>

- [ ] **2.5** Implement XML marker parsing in `agent-adapter`.

<task id="T-205" req="REQ-014a" wave="1" depends="">
  <description>Parse &lt;triumvirate_tool name="ledger_record"&gt;...&lt;/triumvirate_tool&gt; from agent stdout. Extract params. Return structured ToolCallRequest. Handle malformed XML with error.</description>
  <files>agent-adapter/src/markers.rs</files>
  <verify>cargo test -p agent-adapter</verify>
  <reality_test>Feed parser valid XML marker → extracts name + params correctly. Feed malformed XML → returns parse error (not panic). Feed normal text without markers → returns None. A stub always returning None fails the valid-XML extraction.</reality_test>
</task>

- [ ] **2.6** Add prompt injection + parse success metric.

<task id="T-206" req="REQ-014b" wave="2" depends="T-205">
  <description>Inject XML tool instructions into Gemini/Codex startup prompts. Track marker_parse_success_rate Prometheus metric. Degraded warning if &lt;50% over 1hr.</description>
  <files>triumvirate/src/agent_exec.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Spawn Gemini via ask_agent with a prompt. Capture the actual prompt sent to the subprocess. Verify it contains the XML tool instruction text. Prometheus metric marker_parse_success_rate exists. A stub that sends the original prompt without injection fails the content check.</reality_test>
</task>

- [ ] **2.7** Implement lesson CRUD with confidence decay.

<task id="T-207" req="REQ-019,REQ-020-LL,REQ-021" wave="1" depends="T-104,T-105">
  <description>add_lesson, query_lessons (FTS5 + confidence decay at query time), validate_lesson (reset last_validated_at), list_lessons (filter by tags, stale). Decay: confidence = initial * e^(-0.01 * days).</description>
  <files>ledger/src/lessons.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Add lesson with confidence 0.8. Query immediately — confidence ~0.8. Mock time +50 days. Query — confidence ~0.36. Query with min_confidence=0.4 — not found. Validate lesson. Query — confidence back to ~0.8. A stub returning fixed confidence fails the decay check.</reality_test>
</task>

- [ ] **2.8** Add lesson MCP tools.

<task id="T-208" req="REQ-021" wave="2" depends="T-207">
  <description>Register lesson_add, lesson_query, lesson_validate, lesson_list MCP tools.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Call lesson_add via MCP. Call lesson_query via MCP — returns the lesson. Call lesson_validate via MCP. Call lesson_list — shows updated validation time. A stub registering tools without wiring fails the round-trip.</reality_test>
</task>

- [ ] **2.9** Implement auto-lesson creation from compression.

<task id="T-209" req="REQ-022" wave="2" depends="T-116,T-207">
  <description>When compression produces summary with type error_resolution, bug_fix, or architecture_decision, auto-create lesson with confidence 0.6.</description>
  <files>ledger/src/compression.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Ingest events that produce a bug_fix summary. After compression, lessons table contains auto-created lesson with confidence=0.6 and title from the summary. Ingest events that produce an "extractive" summary (not bug_fix). No lesson created. A stub that creates lessons for every summary type fails the selectivity check.</reality_test>
</task>

**Gate:** `ledger_query("test")` returns results. Lesson added, queried after simulated decay, confidence correct.

---

## Phase 3: Fleet Core

**Crates:** `fleet` (new crate)
**REQs:** 028–037, 034a

- [ ] **3.1** Create `fleet` crate.

<task id="T-301" req="REQ-001" wave="0" depends="">
  <description>Create fleet crate Cargo.toml with shared-types, daemon-core, agent-worker, ledger deps</description>
  <files>fleet/Cargo.toml</files>
  <verify>cargo check -p fleet</verify>
  <reality_test>Cargo.toml does NOT contain direct git dependencies (grep for "git" or "Command" in src/). Uses GitOps trait from shared-types. cargo build succeeds.</reality_test>
</task>

- [ ] **3.2** Add fleet + tasks tables to Ledger.

<task id="T-302" req="REQ-031,REQ-034" wave="0" depends="T-104">
  <description>Add fleets and tasks tables to ledger schema. Add reviews table for Phase 4.</description>
  <files>ledger/src/store.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Open DB. SELECT name FROM sqlite_master returns fleets, tasks, reviews tables. Each table has correct columns per BACKEND_STRUCTURE.md schema.</reality_test>
</task>

- [ ] **3.3** Implement real GitOps.

<task id="T-303" req="REQ-003" wave="1" depends="T-100">
  <description>Real GitOps impl using tokio::process::Command for git operations. Lives in triumvirate binary, not fleet crate.</description>
  <files>triumvirate/src/git_ops_impl.rs</files>
  <verify>cargo test -p triumvirate</verify>
  <reality_test>Create temp git repo. Call worktree_add → verify dir exists + is valid git worktree. Call is_clean on clean repo → true. Create uncommitted file → is_clean returns false. Call worktree_remove → dir gone. A stub returning Ok(()) for all methods fails the directory existence check.</reality_test>
</task>

- [ ] **3.4** Implement worktree management.

<task id="T-304" req="REQ-028,REQ-029,REQ-030" wave="1" depends="T-301,T-303">
  <description>Create/remove worktrees via GitOps trait. Verify clean tree before create. Fail with actionable error on dirty tree.</description>
  <files>fleet/src/worktree.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>Clean repo → worktree created successfully at expected path. Dirty repo (uncommitted changes) → returns error containing the word "uncommitted" or "dirty". A stub that always creates fails the dirty-tree rejection.</reality_test>
</task>

- [ ] **3.5** Implement task claiming + dependency resolution.

<task id="T-305" req="REQ-032,REQ-033" wave="1" depends="T-302">
  <description>Atomic task claim via SQLite UPDATE WHERE state='pending' + changes()==1. Dependency graph: tasks with unmet depends_on are not claimable.</description>
  <files>fleet/src/tasks.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>Create T-001 (no deps) and T-002 (depends T-001). List claimable → only T-001. Claim T-001 → success. Claim T-001 again → fails (already claimed). Complete T-001. List claimable → now T-002. Two concurrent claims on same task → exactly one succeeds. A stub with no atomicity allows double-claim.</reality_test>
</task>

- [ ] **3.6** Implement fleet orchestrator (spawn + task delivery).

<task id="T-306" req="REQ-035,REQ-036,REQ-037" wave="2" depends="T-304,T-305">
  <description>fleet_spawn: dry_run returns plan, execute creates worktrees + writes fleet-task.md + sets TRIUMVIRATE_PROJECT_ROOT + spawns agents. Emits progress events to Ledger.</description>
  <files>fleet/src/orchestrator.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>fleet_spawn(dry_run=true) returns plan text containing agent count and HEAD SHA. fleet_spawn(dry_run=false) creates worktree dirs. .triumvirate/fleet-task.md exists in each with correct frontmatter (task_id, fleet_id, assigned_agent). Ledger contains fleet_spawned event. A stub returning fleet_id without creating worktrees fails the dir check.</reality_test>
</task>

- [ ] **3.7** Add fleet MCP tools.

<task id="T-307" req="REQ-035" wave="2" depends="T-306">
  <description>Register fleet_spawn, fleet_status, fleet_task_list, fleet_claim_task, fleet_cancel MCP tools.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Call fleet_spawn(dry_run=true) via MCP → returns plan. Call fleet_spawn(dry_run=false) via MCP → returns fleet_id. Call fleet_status → returns state. A stub registering tools without wiring fails the status check.</reality_test>
</task>

- [ ] **3.8** Implement sequential merge.

<task id="T-308" req="REQ-038" wave="3" depends="T-303,T-306">
  <description>Merge worktrees back to main in task completion order. One at a time via GitOps::merge.</description>
  <files>fleet/src/merge.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>Fleet with 2 agents. Agent-2 completes first, then agent-1. Merge order: agent-2 first, then agent-1. Git log shows correct merge order. A stub merging in creation order (not completion order) fails.</reality_test>
</task>

- [ ] **3.9** Implement conflict detection + notification.

<task id="T-309" req="REQ-039,REQ-039a,REQ-039b" wave="3" depends="T-308">
  <description>On merge conflict: log to Ledger, surface via MCP notification (within 1s, names files + agents), pause merge queue. No auto-resolve.</description>
  <files>fleet/src/merge.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>Create 2 worktrees that modify the same file differently. Merge first → success. Merge second → conflict detected. Ledger contains conflict event naming the file. Merge queue paused (subsequent merge attempt returns "paused"). A stub that silently drops conflicts fails the Ledger check.</reality_test>
</task>

- [ ] **3.10** Implement fleet crash recovery.

<task id="T-310" req="REQ-034a" wave="3" depends="T-302,T-304">
  <description>On daemon startup: scan fleets in non-terminal states. Check PIDs alive. Dead → mark failed, reset tasks to pending, clean worktrees, emit recovery events.</description>
  <files>fleet/src/recovery.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>Set fleet to 'running' in SQLite with fake PIDs (not alive). Run recovery. Fleet state is 'failed'. failure_reason contains "crash recovery". Worktree dirs cleaned. Ledger has recovery event. A stub that leaves stale fleets in 'running' fails the state check.</reality_test>
</task>

**Gate:** Spawn 2-agent fleet. Both get worktrees. Tasks claimed. Work produced. Merge succeeds. Kill daemon mid-fleet → restart → fleet marked failed, worktrees cleaned.

---

## Phase 4: Review

**Crates:** `peer-review` (new crate)
**REQs:** 023–027b, 024a, 040–040b

- [ ] **4.1** Create `peer-review` crate.

<task id="T-401" req="REQ-001" wave="0" depends="">
  <description>Create peer-review crate with shared-types and ledger deps.</description>
  <files>peer-review/Cargo.toml</files>
  <verify>cargo check -p peer-review</verify>
  <reality_test>cargo build succeeds. Cargo.toml has shared-types + ledger only (no daemon-core, no fleet).</reality_test>
</task>

- [ ] **4.2** Reviews table already added in T-302 (Phase 3 Wave 0).

- [ ] **4.3** Implement review logic.

<task id="T-403" req="REQ-023,REQ-024-PR,REQ-024a" wave="1" depends="T-302,T-401">
  <description>ReviewRequest struct. Round-robin assignment to non-author agent. Bounded queue (max TRIUMVIRATE_REVIEW_MAX_INFLIGHT, default 2). 120s timeout → failed. Reviews stored in Ledger.</description>
  <files>peer-review/src/lib.rs</files>
  <verify>cargo test -p peer-review</verify>
  <reality_test>review_request(author=codex) assigns claude or gemini, NEVER codex. Submit 5 requests — only 2 in_progress at once, 3 queued. Mock reviewer that never responds — after 120s, review state=failed. A stub that assigns author as reviewer fails the non-self check.</reality_test>
</task>

- [ ] **4.4** Add review MCP tools.

<task id="T-404" req="REQ-026" wave="2" depends="T-403">
  <description>Register review_request, review_submit, review_status MCP tools.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Call review_request via MCP → returns review_id + assigned reviewer. Call review_submit with verdict=approve → stored in Ledger. Call review_status → shows completed. A stub returning static IDs without Ledger storage fails the persistence check.</reality_test>
</task>

- [ ] **4.5** Integrate review gate into fleet merge.

<task id="T-405" req="REQ-040,REQ-040a" wave="2" depends="T-308,T-403">
  <description>Fleet merge checks review status before each sequential merge. Merge blocked without approval. request_changes pauses queue + surfaces comments.</description>
  <files>fleet/src/merge.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>Fleet with 2 agents. Agent-1 completes, review NOT submitted. Attempt merge → blocked with "pending review" message. Submit approval. Merge proceeds. Submit request_changes for agent-2 → merge paused, comments surfaced. A stub that merges without checking review fails.</reality_test>
</task>

- [ ] **4.6** Implement skip review flag.

<task id="T-406" req="REQ-040b" wave="2" depends="T-405">
  <description>TRIUMVIRATE_FLEET_SKIP_REVIEW=1 bypasses review gate. Skipped reviews logged to Ledger with summary_type=review_skipped.</description>
  <files>fleet/src/merge.rs</files>
  <verify>cargo test -p fleet</verify>
  <reality_test>Set env var. Fleet merge proceeds without review. Ledger contains entry with summary_type="review_skipped". Unset env var. Merge requires review again. A stub that always skips regardless of env fails the env-check.</reality_test>
</task>

- [ ] **4.7** Implement mandatory peer review mode.

<task id="T-407" req="REQ-027b" wave="2" depends="T-403">
  <description>TRIUMVIRATE_REQUIRE_PEER_REVIEW=1 makes review mandatory for ALL agent output, not just fleet merges.</description>
  <files>triumvirate/src/agent_exec.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Set env var. Run ask_agent(codex, "hello"). Output goes through review before returning to user. Unset env var. ask_agent returns immediately without review. A stub that ignores the env var fails the gating check.</reality_test>
</task>

**Gate:** Fleet merge blocked until review approved. Self-review rejected. Skip flag works. Mandatory mode works.

---

## Phase 5: Visibility (Dashboard)

**Crates:** `dashboard` (new — Svelte project)
**REQs:** 041–048, 044a, 045a–045e

- [ ] **5.1** DESIGN_SYSTEM.md already created at `dashboard/DESIGN_SYSTEM.md` (Phase 4 deliverable).

- [ ] **5.2** Scaffold Svelte 5 project.

<task id="T-502" req="REQ-041" wave="0" depends="">
  <description>Scaffold Svelte 5 + Vite 6 + Tailwind 4 + @sveltejs/adapter-static project at dashboard/. npm run build produces dist/.</description>
  <files>dashboard/package.json, dashboard/vite.config.ts, dashboard/svelte.config.js, dashboard/tailwind.config.ts</files>
  <verify>cd dashboard && npm run build</verify>
  <reality_test>npm run build produces dashboard/dist/index.html. File contains &lt;html. dist/ contains JS and CSS files. Build uses --base=./ for relative paths. A scaffold with no adapter-static that produces server-side output fails the static file check.</reality_test>
</task>

- [ ] **5.3** Add WebSocket endpoint.

<task id="T-503" req="REQ-044" wave="1" depends="">
  <description>GET /ws WebSocket upgrade in daemon. Streams agent_state, fleet_progress, ledger_health, review_completed events as JSON.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Start daemon. Connect to ws://localhost:8080/ws (via wscat or test client). Trigger an agent action. Receive JSON event with type field. Disconnect + reconnect — no crash. A stub that accepts the upgrade but never sends events fails the event check.</reality_test>
</task>

- [ ] **5.4** Remove local execution path.

<task id="T-504" req="REQ-044a" wave="1" depends="">
  <description>MCP bridge operates as daemon-proxy only. Remove all local agent execution code paths. Daemon unreachable + auto-start fails → explicit error with recovery command.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Stop daemon. Call ask_agent via MCP bridge. Response is an error containing "daemon" and "triumvirate daemon" (recovery instruction). NOT a successful agent response from local execution. A stub that falls back to local execution fails.</reality_test>
</task>

- [ ] **5.5** Build Sessions view.

<task id="T-505" req="REQ-045,REQ-046" wave="2" depends="T-502,T-503">
  <description>Sessions view: active/recent sessions, agent states (idle/working/stuck), live working-state streaming. Verbosity selector per agent.</description>
  <files>dashboard/src/routes/sessions/+page.svelte, dashboard/src/lib/stores/agents.ts</files>
  <verify>npm run build</verify>
  <reality_test>Load dashboard. Sessions view shows at least one agent state. Change verbosity dropdown → displayed events change. WebSocket delivers agent_state events → UI updates reactively. A stub page with static text fails the reactive update check.</reality_test>
</task>

- [ ] **5.6** Build Fleet view.

<task id="T-506" req="REQ-045a" wave="2" depends="T-502,T-503">
  <description>Fleet view: active fleets, kanban columns (pending/claimed/in_progress/done/failed), worktree status, merge queue.</description>
  <files>dashboard/src/routes/fleet/+page.svelte, dashboard/src/lib/stores/fleet.ts</files>
  <verify>npm run build</verify>
  <reality_test>Load dashboard with active fleet. Kanban shows tasks in correct columns. Agent completes task → card moves from in_progress to done without page refresh. A static kanban with no WebSocket updates fails.</reality_test>
</task>

- [ ] **5.7** Build Ledger view.

<task id="T-507" req="REQ-045b,REQ-047" wave="2" depends="T-502">
  <description>Ledger view: session history, FTS5 search input, compression queue depth, health indicator (green/yellow/red with pulse on red).</description>
  <files>dashboard/src/routes/ledger/+page.svelte, dashboard/src/lib/stores/ledger.ts</files>
  <verify>npm run build</verify>
  <reality_test>Load dashboard. Ledger view shows health indicator (green if healthy). Type search query → results appear from FTS5. Stop sending events for 5 min during active session → indicator turns red with pulse. A static page with no health polling fails the indicator change.</reality_test>
</task>

- [ ] **5.8** Build Lessons view.

<task id="T-508" req="REQ-045c" wave="2" depends="T-502">
  <description>Lesson list with confidence bars (0-1.0). Stale lessons (confidence &lt;0.1) highlighted. Add/validate action buttons.</description>
  <files>dashboard/src/routes/lessons/+page.svelte, dashboard/src/lib/stores/lessons.ts</files>
  <verify>npm run build</verify>
  <reality_test>Load dashboard with lessons. Confidence bars render at correct widths. Stale lesson has red highlight. Click validate → confidence bar updates (API call + re-render). A page listing lessons without confidence bars fails.</reality_test>
</task>

- [ ] **5.9** Build Reviews view.

<task id="T-509" req="REQ-045d" wave="2" depends="T-502">
  <description>Pending reviews with age, review history, approval rate per agent.</description>
  <files>dashboard/src/routes/reviews/+page.svelte, dashboard/src/lib/stores/reviews.ts</files>
  <verify>npm run build</verify>
  <reality_test>Load dashboard with completed reviews. Approval rate shows per-agent percentages. Pending review shows age (time since requested). Review completes → moves from pending to history without refresh. A static list fails the age calculation.</reality_test>
</task>

- [ ] **5.10** Build Metrics view.

<task id="T-510" req="REQ-045e" wave="2" depends="T-502">
  <description>Per-agent token usage, cost per session, latency histograms. Data from Prometheus /metrics endpoint.</description>
  <files>dashboard/src/routes/metrics/+page.svelte</files>
  <verify>npm run build</verify>
  <reality_test>Load dashboard. Metrics view fetches from /metrics. Shows at least one chart/table with agent token data. Values are non-zero after agent activity. A page with no /metrics fetch fails.</reality_test>
</task>

- [ ] **5.11** Configure rust-embed.

<task id="T-511" req="REQ-042" wave="3" depends="T-502">
  <description>Add rust-embed to triumvirate binary. #[derive(RustEmbed)] #[folder = "dashboard/dist"]. Serve at GET / and GET /assets/*. Fallback to index.html for SPA routing.</description>
  <files>triumvirate/src/main.rs, triumvirate/Cargo.toml</files>
  <verify>cargo build -p triumvirate</verify>
  <reality_test>Build dashboard (npm run build). Build binary (cargo build). curl localhost:8080/ returns HTML containing Svelte mount point. curl localhost:8080/assets/some-hash.js returns JavaScript. curl localhost:8080/fleet returns index.html (SPA fallback). A binary without rust-embed returns 404 on GET /.</reality_test>
</task>

**Gate:** Dashboard loads at localhost:8080. All 6 views render. Ledger health indicator shows green. Live agent state streaming works.

---

## Phase 6: Enrichment + Codex Protocol

**Crates:** `shared-types` (extend), `agent-adapter` (extend)
**REQs:** 049–055, 054a

- [ ] **6.1** Add optional fields to OutboxEvent.

<task id="T-601" req="REQ-049,REQ-049a,REQ-049b" wave="1" depends="">
  <description>Add working_state: Option&lt;String&gt;, token_usage: Option&lt;TokenUsage&gt;, tool_name: Option&lt;String&gt; to OutboxEvent. Backward-compatible (serde skip_serializing_if none).</description>
  <files>shared-types/src/lib.rs</files>
  <verify>cargo check -p shared-types</verify>
  <reality_test>Serialize OutboxEvent WITH new fields → JSON contains working_state, token_usage, tool_name. Deserialize OLD JSON (without new fields) → succeeds, new fields are None. A struct without #[serde(skip_serializing_if)] bloats old readers.</reality_test>
</task>

- [ ] **6.2** Populate new fields.

<task id="T-602" req="REQ-050" wave="1" depends="T-601">
  <description>agent_exec.rs populates OutboxEvent fields from ParsedAgentResult.</description>
  <files>triumvirate/src/agent_exec.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Run ask_agent with mock CLI that outputs token usage. OutboxEvent written to outbox.jsonl contains non-None token_usage and tool_name. A stub that never populates the fields leaves them None in the output.</reality_test>
</task>

- [ ] **6.3** Implement CodexAppServerParser.

<task id="T-603" req="REQ-051" wave="1" depends="">
  <description>JSON-RPC 2.0 parser: initialize → initialized → thread/start → turn/start → stream notifications → turn/completed. Line-by-line JSONL from Codex stdout.</description>
  <files>agent-adapter/src/codex_app_server.rs</files>
  <verify>cargo test -p agent-adapter</verify>
  <reality_test>Feed mock Codex app-server JSONL (initialize, thread/start, turn/start, text delta, turn/completed). Parser produces correct ParsedAgentResult with response text and token counts. Feed exec-format JSON → parser returns error (wrong protocol). A stub that parses all input as plain text fails the structured extraction.</reality_test>
</task>

- [ ] **6.4** Handle approval requests + capability probe.

<task id="T-604" req="REQ-052,REQ-054a" wave="2" depends="T-603">
  <description>Detect approval_request events. Probe approval response channel on startup. If functional → JSON-RPC ProceedOnce. If broken → log warning, rely on --full-auto.</description>
  <files>agent-adapter/src/codex_app_server.rs</files>
  <verify>cargo test -p agent-adapter</verify>
  <reality_test>Feed mock approval_request event. Parser detects it and returns ApprovalRequest variant. Probe mock that supports approval → probe returns Ok. Probe mock that returns "method not supported" → probe returns Err. A stub that ignores approval events fails detection.</reality_test>
</task>

- [ ] **6.5** Implement --full-auto flag injection.

<task id="T-605" req="REQ-054" wave="2" depends="">
  <description>When TRIUMVIRATE_CODEX_AUTO_APPROVE=1, append --full-auto to Codex CLI command.</description>
  <files>triumvirate/src/agent_exec.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Set CODEX_AUTO_APPROVE=1. Capture the command spawned for Codex. Args contain --full-auto. Unset env var. Args do NOT contain --full-auto. A stub that always includes the flag regardless of env fails the unset check.</reality_test>
</task>

- [ ] **6.6** Log auto-approved actions to Ledger.

<task id="T-606" req="REQ-055" wave="2" depends="T-604">
  <description>When auto-approve fires (via --full-auto or ProceedOnce), write Ledger entry with summary_type=auto_approved.</description>
  <files>triumvirate/src/agent_exec.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Run Codex with auto-approve. Query Ledger for summary_type="auto_approved". At least one entry exists. Entry contains the action that was approved. A stub that approves without logging fails the Ledger query.</reality_test>
</task>

- [ ] **6.7** Wire CODEX_PROTOCOL env var.

<task id="T-607" req="REQ-053" wave="1" depends="">
  <description>TRIUMVIRATE_CODEX_PROTOCOL env var: "exec" (default) or "app-server". Daemon selects parser based on value.</description>
  <files>mcp-bridge/src/lib.rs</files>
  <verify>cargo test -p mcp-bridge</verify>
  <reality_test>Set CODEX_PROTOCOL=app-server. codex_protocol() returns "app-server". Set CODEX_PROTOCOL=exec. Returns "exec". Unset → returns "exec" (default). Set to "invalid" → returns "exec" (safe fallback). A stub that always returns "exec" fails the app-server check.</reality_test>
</task>

**Gate:** Codex connects via app-server. Auto-approve fires. Fallback to exec works. OutboxEvents contain token_usage.

---

## Phase 7: Polish (GC)

**REQs:** 056–058

- [ ] **7.1** Implement `gc()`.

<task id="T-701" req="REQ-056,REQ-057" wave="1" depends="T-104">
  <description>Delete events >30 days old with no linked summary. Events WITH summaries retained. Clear acknowledged dead-drop tickets >7 days. Return GcResult with counts.</description>
  <files>ledger/src/gc.rs</files>
  <verify>cargo test -p ledger</verify>
  <reality_test>Insert event 31 days ago (mock time) with no summary. Insert event 31 days ago WITH linked summary. Run gc(). First event deleted. Second event still exists. Dead-drop ticket 8 days old → deleted. GcResult shows correct counts. A stub that deletes ALL old events regardless of summary fails the retention check.</reality_test>
</task>

- [ ] **7.2** Add `ledger_gc()` MCP tool.

<task id="T-702" req="REQ-057" wave="1" depends="T-701">
  <description>Register ledger_gc MCP tool. Returns events_scanned, events_deleted, space_reclaimed.</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Call ledger_gc via MCP. Response contains events_scanned, events_deleted, space_reclaimed_bytes fields. After GC, DB file size is smaller or equal. A stub returning static zeros fails if stale data exists.</reality_test>
</task>

- [ ] **7.3** Add startup GC.

<task id="T-703" req="REQ-058" wave="1" depends="T-701">
  <description>On daemon startup, run GC if last_gc >24h ago AND no active fleets (fleets table has no non-terminal states).</description>
  <files>triumvirate/src/main.rs</files>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>Set last_gc to 25h ago. No active fleets. Start daemon. GC runs (stale events deleted). Set fleet to 'running'. Restart daemon. GC does NOT run. A stub that always runs GC regardless of fleet state fails the active-fleet check.</reality_test>
</task>

**Gate:** GC deletes stale events. Space reclaimed. Active fleet blocks GC.
