# Backend Structure — Triumvirate v2.2

---

## SQLite Schema

### events
```sql
CREATE TABLE events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    event_type TEXT NOT NULL,  -- SessionStart, PostToolUse, Stop, SessionEnd
    sequence INTEGER NOT NULL,
    timestamp TEXT NOT NULL,   -- ISO 8601
    payload_json TEXT NOT NULL,
    compression_state TEXT NOT NULL DEFAULT 'pending',  -- pending, running, done, failed
    compression_heartbeat TEXT,  -- ISO 8601, updated every 30s while running
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(session_id, event_type, sequence)  -- idempotency key
);
CREATE INDEX idx_events_session ON events(session_id);
CREATE INDEX idx_events_compression ON events(compression_state) WHERE compression_state IN ('pending', 'running');
```

### summaries
```sql
CREATE TABLE summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id INTEGER REFERENCES events(id),
    title TEXT NOT NULL,
    narrative TEXT NOT NULL,
    facts_json TEXT,        -- JSON array of strings
    concepts_json TEXT,     -- JSON array of strings (tags)
    affected_files_json TEXT, -- JSON array of file paths
    summary_type TEXT NOT NULL, -- extractive, llm_abstraction, error_resolution, bug_fix, architecture_decision, auto_approved, review_skipped, session_narrative
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE VIRTUAL TABLE summaries_fts USING fts5(title, narrative, facts_json, content=summaries, content_rowid=id);
```

### sessions
```sql
CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    branch TEXT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    event_count INTEGER NOT NULL DEFAULT 0,
    summary_count INTEGER NOT NULL DEFAULT 0
);
```

### health
```sql
CREATE TABLE health (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    last_event_id INTEGER,
    db_size_bytes INTEGER,
    spool_size_bytes INTEGER,
    queue_depth INTEGER,
    status TEXT NOT NULL  -- healthy, degraded, dead
);
```

### lessons
```sql
CREATE TABLE lessons (
    lesson_id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    source_session_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_validated_at TEXT NOT NULL DEFAULT (datetime('now')),
    initial_confidence REAL NOT NULL DEFAULT 0.8,
    tags_json TEXT,      -- JSON array of strings
    req_ids_json TEXT    -- JSON array of strings
);
CREATE VIRTUAL TABLE lessons_fts USING fts5(title, body, tags_json, content=lessons, content_rowid=lesson_id);
```

### tasks (fleet)
```sql
CREATE TABLE tasks (
    task_id TEXT PRIMARY KEY,
    fleet_id TEXT NOT NULL REFERENCES fleets(fleet_id),
    title TEXT NOT NULL,
    description TEXT,
    assigned_agent TEXT,
    state TEXT NOT NULL DEFAULT 'pending',  -- pending, claimed, in_progress, done, failed, blocked
    depends_on TEXT,     -- JSON array of task_ids
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);
CREATE INDEX idx_tasks_fleet ON tasks(fleet_id);
CREATE INDEX idx_tasks_state ON tasks(state);
```

### fleets
```sql
CREATE TABLE fleets (
    fleet_id TEXT PRIMARY KEY,
    task_description TEXT NOT NULL,
    agent_composition TEXT NOT NULL,  -- JSON: {"claude": 2, "codex": 1}
    source_project_root TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'spawning',  -- spawning, running, merging, done, failed, recovery_required
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT,
    failure_reason TEXT
);
```

### reviews
```sql
CREATE TABLE reviews (
    review_id TEXT PRIMARY KEY,
    fleet_id TEXT,
    author_agent TEXT NOT NULL,
    reviewer_agent TEXT,
    artifact TEXT NOT NULL,      -- diff content or file path
    review_type TEXT NOT NULL,   -- code, architecture, decision
    verdict TEXT,                -- approve, request_changes, reject
    comments TEXT,
    requested_at TEXT NOT NULL DEFAULT (datetime('now')),
    reviewed_at TEXT,
    state TEXT NOT NULL DEFAULT 'pending'  -- pending, in_progress, done, failed, timeout
);
CREATE INDEX idx_reviews_state ON reviews(state);
```

---

## Traits (shared-types)

### GitOps
```rust
#[async_trait]
pub trait GitOps: Send + Sync {
    async fn worktree_add(&self, path: &Path, branch: &str) -> Result<()>;
    async fn worktree_remove(&self, path: &Path) -> Result<()>;
    async fn is_clean(&self) -> Result<bool>;
    async fn current_head(&self) -> Result<String>;
    async fn merge(&self, branch: &str) -> Result<MergeResult>;
    async fn diff(&self, branch: &str) -> Result<String>;
    async fn rev_parse_toplevel(&self, cwd: &Path) -> Result<PathBuf>;
}

pub enum MergeResult {
    Success,
    Conflict { files: Vec<String> },
}
```

### LedgerStore (ledger crate public API)
```rust
pub struct LedgerStore { /* SQLite connection pool */ }

impl LedgerStore {
    pub fn open(project_root: PathBuf) -> Result<Self>;
    pub fn ingest_event(&self, event: RawEvent) -> Result<()>;
    pub fn drain_spool(&self, spool_dir: &Path) -> Result<DrainResult>;
    pub fn query(&self, query: &str, limit: usize) -> Result<Vec<Summary>>;
    pub fn get_session(&self, session_id: &str) -> Result<SessionDetail>;
    pub fn record(&self, record: ManualRecord) -> Result<()>;
    pub fn health(&self) -> Result<HealthStatus>;
    pub fn add_lesson(&self, lesson: NewLesson) -> Result<i64>;
    pub fn query_lessons(&self, query: &str, min_confidence: f64) -> Result<Vec<Lesson>>;
    pub fn validate_lesson(&self, lesson_id: i64) -> Result<()>;
    pub fn gc(&self) -> Result<GcResult>;
}
```

Note: `LedgerStore::open()` takes an absolute `PathBuf`. It does NOT resolve project roots (REQ-002a).

---

## HTTP Endpoints (new in v2.2)

| Method | Path | Body | Purpose | Phase |
|--------|------|------|---------|-------|
| POST | `/ledger/wake` | `{"project_root": "..."}` | Wake spool drainer | 1 |
| GET | `/ledger/health` | — | Health status | 1 |
| GET | `/ws` | — | WebSocket upgrade | 5 |

All existing endpoints unchanged. Bearer token auth on all routes.

---

## Spool Directory Layout

```
<project>/.triumvirate/
    ledger.db           # SQLite WAL database
    ledger.db-wal       # WAL file (auto-managed)
    ledger.db-shm       # Shared memory (auto-managed)
    spool/
        event-1712534400-12345-8192.ndjson   # Ingested events (processed → deleted)
        event-1712534401-12345-4096.tmp      # In-flight write (not yet renamed)
```

---

## Compression Pipeline

```
Spool file → daemon drain → events table (compression_state=pending)
                                    ↓
                          compression worker (Tokio task)
                                    ↓
                    Tier 0: local extractive summary (always)
                                    ↓
                    [if 3+ errors OR 5+ edits OR ledger_record]
                                    ↓
                    Tier 1: LLM abstraction (budget-capped)
                                    ↓
                          summaries table + FTS5 index
                                    ↓
                    [if summary_type in (error_resolution, bug_fix, architecture_decision)]
                                    ↓
                          auto-create lesson (confidence 0.6)
```

---

## WebSocket Event Stream (REQ-044)

```json
{"type": "agent_state", "agent": "claude-1", "state": "working", "tool": "Edit"}
{"type": "fleet_progress", "fleet_id": "abc", "event": "task_completed", "agent": "codex-1", "task_id": "T-003"}
{"type": "ledger_health", "status": "healthy", "events_5min": 23, "queue_depth": 0}
{"type": "review_completed", "review_id": "xyz", "verdict": "approve", "reviewer": "gemini"}
```
