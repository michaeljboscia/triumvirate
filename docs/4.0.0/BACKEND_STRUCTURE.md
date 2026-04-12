# Pantheon v4.0 — Backend Structure

**Spec:** specs/PANTHEON_V4.md  
**PRD:** docs/4.0.0/PRD.md  

---

## Two Backends

1. **Daemon backend (v3.9.0)** — existing Rust daemon with new endpoints, lineage tracking, and event replay
2. **Tauri backend (v4.0.0)** — Rust code in `pantheon/src-tauri/` that manages PTYs, connects to daemon, scans processes

Pantheon talks to the daemon over its PUBLIC API only. No shared state, no shared memory, no crate-level coupling beyond `shared-types`.

---

## v3.9.0 Daemon Additions

### SQLite Schema Changes

```sql
-- Add lineage columns to sessions table
ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;
ALTER TABLE sessions ADD COLUMN root_session_id TEXT;
ALTER TABLE sessions ADD COLUMN pantheon_session_id TEXT;

-- Index for hierarchy queries
CREATE INDEX idx_sessions_parent ON sessions(parent_session_id);
CREATE INDEX idx_sessions_root ON sessions(root_session_id);
CREATE INDEX idx_sessions_pantheon ON sessions(pantheon_session_id);
```

### Lineage Capture

When the daemon receives an MCP request that dispatches a worker:

```
1. Read X-Pantheon-Session-Id from HTTP header (proxy transport)
   OR read _meta.pantheon.session_id from MCP initialize params (stdio transport)
2. Look up the MCP session's canonical session_id
3. Set parent_session_id = caller's session_id
4. Set root_session_id = caller's root_session_id (or caller's session_id if caller has no root)
5. Persist to SQLite
```

### REST Endpoints

#### GET /api/workers
Returns all active sessions and workers with lineage.

```json
{
  "workers": [
    {
      "session_id": "sess-abc123",
      "agent": "codex",
      "name": "codex-worker-1",
      "status": "working",
      "task_id": "T-001",
      "parent_session_id": "sess-main-1",
      "root_session_id": "sess-main-1",
      "pantheon_session_id": "uuid-from-pantheon",
      "cwd": "/Users/mikeboscia/projects/triumvirate",
      "started_at": "2026-04-11T22:00:00Z",
      "elapsed_ms": 45000
    }
  ]
}
```

Auth: Bearer token from `~/.triumvirate/daemon.token`.

#### GET /api/fleet
Returns all active ABE builds.

```json
{
  "builds": [
    {
      "build_id": "build-001",
      "task_count": 6,
      "completed": 3,
      "failed": 0,
      "in_progress": 2,
      "queued": 1,
      "tasks": [
        {
          "task_id": "T-001",
          "status": "committed",
          "files": ["src/auth.rs"],
          "worker_session_id": "sess-abc123",
          "elapsed_ms": 42000,
          "commit_sha": "a1b2c3d"
        }
      ]
    }
  ]
}
```

#### GET /api/fleet/{build_id}

Returns a single `FleetBuild` by its `build_id`. Used by Pantheon's sidebar
drill-down when the user clicks a build in the overview list.

- **200 OK** + `FleetBuild` JSON on hit
- **404 Not Found** when `build_id` is absent from `fleet_v2_states`
- **401 Unauthorized** when the bearer header is missing or wrong

Response body on hit is the same `FleetBuild` shape shown inside `/api/fleet`'s
`builds[]` array. (NOT wrapped in a `FleetResponse`.) Auth: bearer token from
`~/.triumvirate/daemon.token`. Axum 0.8 path syntax mandates `{build_id}` —
the old colon-prefix `/:build_id` panics at router construction and must
never appear in source.

#### GET /api/state

Full state snapshot for reconnect. Matches the frozen `shared_types::api::StateResponse`
type from T-002 and does **not** carry a separate `sessions` array — named MCP
sessions are exposed via the existing `/session/list` route, while ABE-dispatched
workers (the only entries that appear in `/api/state.workers`) come from
`TaskTracker::snapshot_workers()` on the v3.9.0 DaemonState.

```json
{
  "version": "3.9.0",
  "uptime_ms": 3600000,
  "workers": [],
  "fleet": [],
  "last_event_seq": 1542
}
```

Fields:

- `version` — daemon semver as a `String` (source: `daemon_core::VERSION.to_string()`)
- `uptime_ms` — milliseconds since `DaemonState.started_at` (an `Instant` captured in `run_daemon`)
- `workers` — `Vec<WorkerInfo>` aggregated from `state.abe_tasks.snapshot_workers().await` (ABE workers only; MCP sessions are out-of-scope for this endpoint, reachable via `/session/list`)
- `fleet` — `Vec<FleetBuild>` from `state.fleet_v2_states.lock().await.values().cloned().collect()`
- `last_event_seq` — `u64` from `state.last_event_seq.load(Ordering::Relaxed)`; used by clients on reconnect to send `{"action":"subscribe","last_seq":N}` to `/ws/v2`

### Event Replay Ring Buffer

The actual production type lives in `daemon/crates/daemon-core/src/replay.rs`
(added by T-006). Public API:

```rust
pub struct EventReplayBuffer {
    // Arc<RwLock<VecDeque<AgentStreamEvent>>> internally — Clone-safe.
    // DEFAULT_CAPACITY = 1000 events (~200 KB at ~200 bytes/event).
}

impl EventReplayBuffer {
    pub fn new(capacity: usize) -> Self;
    pub fn default_capacity() -> Self;       // DEFAULT_CAPACITY
    pub fn push(&self, event: AgentStreamEvent);
    pub fn replay_since(&self, last_seq: u64) -> ReplayResult;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn seq_range(&self) -> Option<(u64, u64)>;
}

pub enum ReplayResult {
    /// Client's lastSeq is within buffer range. Events to replay.
    /// Empty vec means "you're caught up" (last_seq >= newest).
    Events(Vec<AgentStreamEvent>),
    /// Client's lastSeq is older than the buffer's oldest event.
    /// Client must do a full /api/state refresh.
    OutOfRange {
        /// The oldest seq currently in the buffer. Client's last_seq < this.
        oldest_seq: u64,
    },
}
```

**WebSocket handshake for /ws/v2** (added by T-009 — legacy /ws is unchanged):
the client sends `{"action": "subscribe", "last_seq": N}` as its first message.
The server subscribes to the broadcast channel FIRST (subscribe-before-read
race fix), THEN reads the replay buffer snapshot, THEN branches on
`ReplayResult`, THEN dedupes the live tail by comparing each incoming event's
`seq()` against `max_sent`.

**Wire format consistency**: Both historical replay frames AND live tail frames
use the same `daemon_core::encode_ws_event("agent_stream", payload)` envelope
shape — `{"type":"agent_stream","ts_ms":N,"payload":<AgentStreamEvent>}`. The
`ReplayResponse` ack (`{"replay":"ok"}` or `{"replay":"out_of_range","oldest_seq":N}`)
is sent as a bare JSON object without the envelope — clients distinguish by
the presence of the top-level `"replay"` field.

**RecvError::Lagged** (broadcast buffer overflow): close the connection. Client
reconnects with its current `last_seq` and the handshake starts over. Do NOT
try to recover in place — the canonical pattern is close-and-retry.

### WorkerLifecycle Event

Added to `shared-types/src/streaming.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "event_type")]
pub enum AgentStreamEvent {
    // ... existing variants ...
    
    WorkerLifecycle {
        lifecycle: WorkerLifecycleType,
        agent: String,
        session_name: String,
        task_id: Option<String>,
        parent_session_id: Option<String>,
        root_session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        commit_sha: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
        elapsed_ms: Option<u64>,
        seq: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerLifecycleType {
    Spawned,
    Completed,
    Failed,
}
```

### PID File Management

```rust
// In daemon startup (main.rs or daemon-core)
use pidfile_rs::Pidfile;

fn setup_pid_file() -> anyhow::Result<Pidfile> {
    let pid_path = dirs::home_dir()
        .unwrap()
        .join(".triumvirate")
        .join("daemon.pid");
    
    Pidfile::new(&pid_path, 0o600)
        .context("Another daemon instance is running")
}
```

Pantheon reads PID file to check if daemon is running before spawning. Verifies PID via `libproc` before sending signals.

### PANTHEON_SESSION_ID Capture

In MCP handler (proxy transport):
```rust
// Read from HTTP header
let pantheon_id = request.headers()
    .get("X-Pantheon-Session-Id")
    .and_then(|v| v.to_str().ok())
    .map(String::from);
```

In MCP handler (stdio transport):
```rust
// Read from initialize params._meta
let pantheon_id = init_params
    .get("_meta")
    .and_then(|m| m.get("pantheon.session_id"))
    .and_then(|v| v.as_str())
    .map(String::from);
```

---

## v4.0.0 Tauri Backend

### Tauri Commands (IPC)

```rust
// PTY management
#[tauri::command]
async fn create_terminal(app: AppHandle, cwd: String) -> Result<String, String>;

#[tauri::command]
async fn write_to_terminal(terminal_id: String, data: Vec<u8>) -> Result<(), String>;

#[tauri::command]
async fn resize_terminal(terminal_id: String, rows: u16, cols: u16) -> Result<(), String>;

#[tauri::command]
async fn close_terminal(terminal_id: String) -> Result<(), String>;

// Daemon connection
#[tauri::command]
async fn get_daemon_status(app: AppHandle) -> Result<DaemonStatus, String>;

#[tauri::command]
async fn start_daemon(app: AppHandle) -> Result<(), String>;

// Process scanning
#[tauri::command]
async fn scan_processes() -> Result<Vec<AgentProcess>, String>;

#[tauri::command]
async fn kill_process(pid: u32) -> Result<(), String>;

// Preferences
#[tauri::command]
async fn get_claude_binary_path() -> Result<String, String>;
```

### Tauri Events (Backend → Frontend)

```rust
// PTY output
app.emit("pty-data", PtyData { terminal_id, bytes });

// PTY exit
app.emit("pty-exit", PtyExit { terminal_id, exit_code, signal });

// Daemon state change
app.emit("daemon-state", DaemonState { state, version });
```

### Process Scanner

```rust
pub struct AgentProcess {
    pub pid: u32,
    pub agent_type: String, // "claude", "gemini", "codex"
    pub cwd: String,
    pub physical_footprint_mb: u64,
    pub command: String,
    pub idle_seconds: u64,
}

pub fn scan_agent_processes() -> Vec<AgentProcess> {
    // 1. Get all processes via sysinfo
    // 2. Read full command-line args (KERN_PROCARGS2)
    // 3. Filter: command contains "claude", "gemini", or "codex" in argv
    // 4. Get cwd via proc_pidinfo(PROC_PIDVNODEPATHINFO)
    // 5. Get Physical Footprint via footprint/vmmap (or sysinfo equivalent)
    // 6. Exclude PIDs that match Pantheon's own PTY children
    // 7. Exclude PIDs that match daemon-managed sessions
}
```

### WebSocket Client

```rust
pub struct DaemonClient {
    ws_tx: mpsc::Sender<String>,
    http_base: String,
    token: String,
    last_seq: AtomicU64,
}

impl DaemonClient {
    pub async fn connect(&mut self) -> Result<()>;
    pub async fn reconnect(&mut self) -> Result<()>;
    pub async fn get_workers(&self) -> Result<Vec<Worker>>;
    pub async fn get_fleet(&self) -> Result<Fleet>;
    pub async fn get_state(&self) -> Result<FullState>;
    pub async fn get_tokens(&self) -> Result<TokenSummary>;
}
```

The DaemonClient runs in a background Tokio task. Events are forwarded to the frontend via Tauri's event system. The frontend subscribes via `listen()`.

---

## Auth

| Component | Mechanism |
|---|---|
| Daemon REST endpoints | Bearer token from `~/.triumvirate/daemon.token` |
| Daemon WebSocket | Token sent in initial subscribe message |
| Pantheon → Daemon | Reads token from file on startup |
| Pantheon PTY → Claude Code | No auth (local child process) |
| Claude Code → Daemon MCP | Existing MCP auth (proxy mode or stdio) |
