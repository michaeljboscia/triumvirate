# v3.1 MCP Consolidation — Backend Structure

**Spec:** `specs/MCP_CONSOLIDATION.md`

---

## Crate Architecture (Target State)

### triumvirate (binary crate) — ~300 lines

Startup wiring only. No tool handlers. No route handlers. No business logic.

```rust
// main.rs — target state
fn main() -> anyhow::Result<()> {
    // 1. Parse CLI args (clap)
    // 2. Load config
    // 3. Init tracing (tracing_setup.rs)
    // 4. Build DaemonState (from daemon-core)
    // 5. Build ObservabilityBus
    // 6. Build McpBridge (from mcp-tools, with ObservabilityBus)
    // 7. Spawn MCP stdio transport (from mcp-bridge)
    // 8. Spawn HTTP server (from daemon-http)
    // 9. Spawn background tasks (ledger drain, session reaper)
    // 10. Await shutdown signal
}
```

### mcp-tools crate — Tool Handlers

**Dependency:** mcp-bridge, daemon-core, agent-adapter, fleet, ledger, peer-review, shared-types, fallback-outbox

#### lib.rs — McpBridge + Registration

```rust
pub struct McpBridge {
    // Narrowed interfaces — NOT a God Object
    pub sessions: SessionStore,         // Arc<Mutex<HashMap<String, SessionState>>>
    pub sessions_file: Option<PathBuf>,
    pub fleet_states: FleetStateStore,  // Arc<Mutex<HashMap<String, FleetStatusResponse>>>
    pub abe_tasks: TaskTracker,
    pub bus: ObservabilityBus,          // NEW: metrics + ws_events
}

// tool_router registers ALL tools including aliases
// Each module's tools are registered here via helper functions
```

#### inter_agent.rs — Session Management

**Receives:** `&SessionStore`, `&AgentExecutor`, `&ObservabilityBus`

| Function | MCP Tool Name | Line in current main.rs |
|----------|--------------|------------------------|
| `spawn_session` | `spawn_session` | 258 |
| `ask_session` | `ask_session` | 298 |
| `dismiss_session` | `dismiss_session` | 342 |
| `list_sessions` | `list_sessions` | 370 |
| `ask_agent` | `ask_agent` | 209 |
| `get_status` | `get_status` | 390 |
| `daemon_health` | `daemon_health` | 426 |

#### abe.rs — Build Enforcement

**Receives:** `&TaskTracker`, `&SessionStore`, `&ObservabilityBus`

| Function | MCP Tool Name | Line |
|----------|--------------|------|
| `dispatch_codex` | `dispatch_codex` | 752 |
| `dispatch_codex_worktree` | `dispatch_codex_worktree` | 859 |
| `get_task_status` | `get_task_status` | 1125 |
| `get_task_output` | `get_task_output` | 1137 |
| `cancel_task` | `cancel_task` | 1149 |

#### fleet.rs — Fleet Operations

**Receives:** `&FleetStateStore`, `&SessionStore`, `&ObservabilityBus`

| Function | MCP Tool Name | Line |
|----------|--------------|------|
| `fleet_spawn` | `fleet_spawn` | 1161 |
| `fleet_status` | `fleet_status` | 1223 |
| `fleet_task_list` | `fleet_task_list` | 1236 |
| `fleet_claim_task` | `fleet_claim_task` | 1261 |
| `fleet_cancel` | `fleet_cancel` | 1279 |

#### knowledge.rs — Ledger, Lessons, Memory, Scratchpad, Outbox

**Receives:** `&LedgerStoreFactory`, `&MemoryStore`, `&OutboxStore`, `&ObservabilityBus`

| Function | MCP Tool Name | Line |
|----------|--------------|------|
| `memory_write` | `memory_write` | 434 |
| `memory_read` | `memory_read` | 460 |
| `scratchpad_write` | `scratchpad_write` | 484 |
| `scratchpad_list` | `scratchpad_list` | 502 |
| `outbox_recent` | `outbox_recent` | 521 |
| `fallback_list` | `fallback_list` | 538 |
| `fallback_ack` | `fallback_ack` | 557 |
| `fallback_gc` | `fallback_gc` | 568 |
| `ledger_health` | `ledger_health` | 584 |
| `ledger_query` | `ledger_query` | 596 |
| `ledger_session` | `ledger_session` | 617 |
| `ledger_record` | `ledger_record` | 638 |
| `ledger_gc` | `ledger_gc` | 655 |
| `lesson_add` | `lesson_add` | 671 |
| `lesson_query` | `lesson_query` | 689 |
| `lesson_validate` | `lesson_validate` | 710 |
| `lesson_list` | `lesson_list` | 730 |

#### review.rs — Peer Review

**Receives:** `&ReviewEngine`, `&ObservabilityBus`

| Function | MCP Tool Name | Line |
|----------|--------------|------|
| `review_request` | `review_request` | 1289 |
| `review_submit` | `review_submit` | 1316 |
| `review_status` | `review_status` | 1334 |

#### gemini_query.rs — Direct Gemini

**Receives:** `&SessionStore`, `&ObservabilityBus`

| Function | MCP Tool Name | Line |
|----------|--------------|------|
| `query_gemini` | `query_gemini` | 1051 |
| `query_gemini_review` | `query_gemini_review` | 1078 |

#### aliases.rs — Backwards Compatibility

**Receives:** same narrowed interfaces as the target module

| Alias | Routes To | Parameter Mapping |
|-------|-----------|-------------------|
| `spawn_daemon` | `spawn_session` | `target` → `agent`, preserves `session_name`, `cwd` |
| `ask_daemon` | `ask_session` | `daemon_id` → `name`, `message` passthrough |
| `dismiss_daemon` | `dismiss_session` | `daemon_id` → `name` |
| `list_daemons` | `list_sessions` | `target` filter → `agent` filter |
| `send_message` | `ask_session` | `target`+`question` → `name`+`message` (synchronous) |
| `get_response` | returns deprecation notice | "Use ask_session directly" |
| `list_jobs` | `get_status` | shape mapping |
| `code_review` | `review_request` | `diff`+`context` → review schema |

Each alias:
1. Logs `tracing::info!("tool_alias", old_name, new_name)`
2. Maps parameters from TS schema to Rust schema
3. Delegates to the canonical handler
4. Preserves `daemon_id` prefix convention (`gd_`/`cd_`)

---

## ObservabilityBus

```rust
pub struct ObservabilityBus {
    pub metrics: Arc<DaemonMetrics>,
    pub ws_events: broadcast::Sender<String>,
}

impl ObservabilityBus {
    pub fn publish_event(&self, event_type: &str, payload: serde_json::Value) {
        let msg = encode_ws_event(event_type, payload);
        let _ = self.ws_events.send(msg);
    }
}
```

Constructed once in `main()`. Cloned (via `Arc`) into:
- `McpBridge` (mcp-tools)
- `DaemonState` (daemon-http)
- Background tasks that need to emit events

### DaemonMetrics (relocated)

Currently in `main.rs:1446-1530`. Moves to a shared location (either `daemon-core` or a dedicated `metrics` module in `mcp-tools`). All 12 existing Prometheus metrics preserved. New metrics added in v3.2.

---

## daemon-http Crate — HTTP Routes

All `*_route` async functions extracted from main.rs. Each route:
1. Extracts `State(DaemonState)` from Axum
2. Calls domain crate function directly
3. Returns HTTP response

No route calls into `mcp-tools`. Both MCP and HTTP are parallel presentation layers over shared domain crates.

### Routes (19 handlers + WebSocket + Dashboard + Metrics)

```
POST /ask-agent          → ask_agent_route (1739)
POST /ledger/wake        → ledger_wake_route (1822)
GET  /ledger/health      → ledger_health_route (1890)
POST /ledger/query       → ledger_query_route (1942)
POST /ledger/session     → ledger_session_route (1987)
POST /ledger/record      → ledger_record_route (2032)
POST /ledger/gc          → ledger_gc_route (2077)
POST /lesson/add         → lesson_add_route (2124)
POST /lesson/query       → lesson_query_route (2168)
POST /lesson/validate    → lesson_validate_route (2212)
GET  /lesson/list        → lesson_list_route (2256)
POST /memory/write       → memory_write_route (2393)
POST /memory/read        → memory_read_route (2424)
POST /scratchpad/write   → scratchpad_write_route (2452)
GET  /scratchpad/list    → scratchpad_list_route (2474)
GET  /outbox/recent      → outbox_recent_route (2498)
GET  /fallback/list      → fallback_list_route (2520)
POST /fallback/ack       → fallback_ack_route (2544)
POST /fallback/gc        → fallback_gc_route (2567)
GET  /ws                 → ws_route (1586)
GET  /metrics            → metrics_route (1655)
GET  /health             → health (1700)
GET  /status             → status (1715)
GET  /                   → dashboard_root_route (1637)
GET  /*path              → dashboard SPA routes (1642, 1648)
```
