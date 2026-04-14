# Pantheon v4.0 — Implementation Plan

**Spec:** specs/PANTHEON_V4.md  
**PRD:** docs/4.0.0/PRD.md  
**Backend:** docs/4.0.0/BACKEND_STRUCTURE.md  

---

## Build Overview

Two releases, strict order: v3.9.0 daemon backend FIRST, v4.0.0 Tauri app SECOND. The Tauri app consumes v3.9.0's API surface — building against APIs that don't exist yet is forbidden.

| Wave | Release | Scope | Tasks |
|------|---------|-------|-------|
| 0 | v3.9.0 | Contracts — shared types, API schemas | 3 |
| 1 | v3.9.0 | Daemon core — lineage, PID file, replay buffer | 4 |
| 2 | v3.9.0 | Daemon HTTP — REST endpoints, WebSocket upgrade | 3 |
| 3 | v3.9.0 | Daemon integration tests + release | 2 |
| 4 | v4.0.0 | Tauri scaffold — app shell, window management, menubar | 3 |
| 5 | v4.0.0 | Terminal panels — PTY, xterm.js, Claude Code integration | 4 |
| 6 | v4.0.0 | Sidebar + status area — daemon client, worker hierarchy, metrics | 4 |
| 7 | v4.0.0 | Polish — dark mode, persistence, notifications, process scanner | 4 |
| 8 | v4.0.0 | Build + release — .dmg, install script, CLI symlink | 2 |

**Total: 29 tasks across 9 waves.**

---

## Wave 0 — Contracts (v3.9.0)

<task id="T-001" req="REQ-010" wave="0" depends="">
  <description>Add WorkerLifecycle variant to AgentStreamEvent in shared-types</description>
  <files>daemon/crates/shared-types/src/streaming.rs</files>
  <scope_out>Do not modify existing AgentStreamEvent variants. Do not change serialization format of existing variants.</scope_out>
  <tools>cargo check -p shared-types, cargo test -p shared-types</tools>
  <verify>cargo check -p shared-types</verify>
  <reality_test>Serialize a WorkerLifecycle::Spawned event to JSON, deserialize it back, assert all fields round-trip including parent_session_id and root_session_id.</reality_test>
  <done_when>WorkerLifecycle variant with Spawned/Completed/Failed sub-types exists, serializes with event_type tag, includes parent_session_id, root_session_id, task_id, and seq fields. All existing tests still pass.</done_when>
</task>

<task id="T-002" req="REQ-010" wave="0" depends="">
  <description>Define REST API response types in shared-types</description>
  <files>daemon/crates/shared-types/src/api.rs, daemon/crates/shared-types/src/lib.rs</files>
  <scope_out>Do not modify existing types. New file api.rs only.</scope_out>
  <tools>cargo check -p shared-types, cargo test -p shared-types</tools>
  <verify>cargo check -p shared-types</verify>
  <reality_test>Create WorkersResponse, FleetResponse, StateResponse structs. Serialize example data to JSON, verify field names match BACKEND_STRUCTURE.md API contracts exactly.</reality_test>
  <done_when>API response types defined, documented, JSON-serializable, and tested with example payloads matching the documented contracts.</done_when>
</task>

<task id="T-003" req="REQ-010" wave="0" depends="">
  <description>Add parent_session_id, root_session_id, pantheon_session_id columns to sessions SQLite schema</description>
  <files>daemon/crates/daemon-core/src/lib.rs, daemon/crates/daemon-core/src/migrations.rs (or equivalent)</files>
  <scope_out>Do not modify existing column definitions. ALTER TABLE ADD COLUMN only. Do not change existing queries.</scope_out>
  <tools>cargo check -p daemon-core, cargo test -p daemon-core</tools>
  <verify>cargo check -p daemon-core</verify>
  <reality_test>Start daemon, verify new columns exist in SQLite with .schema. Insert a session with parent_session_id set, query it back, verify the value persists across daemon restart.</reality_test>
  <done_when>Three new nullable TEXT columns on sessions table. Indexes created. Existing sessions with NULL values for new columns work without errors.</done_when>
</task>

---

## Wave 1 — Daemon Core (v3.9.0)

<task id="T-004" req="REQ-010,REQ-033" wave="1" depends="T-003">
  <description>Implement lineage capture — read PANTHEON_SESSION_ID from MCP _meta and HTTP headers, set parent_session_id on dispatched workers</description>
  <files>daemon/crates/triumvirate/src/http_mcp.rs, daemon/crates/triumvirate/src/agent_exec.rs, daemon/crates/daemon-core/src/lib.rs</files>
  <scope_out>Do not change MCP protocol handling for non-Pantheon clients. Do not modify ABE orchestrator logic. Only add lineage field population.</scope_out>
  <tools>cargo check, cargo test</tools>
  <verify>cargo check</verify>
  <reality_test>Send an MCP initialize request with _meta.pantheon.session_id set. Dispatch a worker from that session. Query the worker's record — verify parent_session_id matches the caller's session and root_session_id is set. Repeat via HTTP proxy with X-Pantheon-Session-Id header.</reality_test>
  <done_when>Workers dispatched from a Pantheon-linked session automatically inherit parent_session_id and root_session_id. Workers dispatched without Pantheon context have NULL lineage fields.</done_when>
</task>

<task id="T-005" req="REQ-019" wave="1" depends="">
  <description>Implement PID file management with flock locking</description>
  <files>daemon/crates/daemon-core/src/pid.rs, daemon/crates/triumvirate/src/main.rs, daemon/Cargo.toml</files>
  <scope_out>Do not change daemon startup sequence beyond adding PID file creation. Do not add launchd integration.</scope_out>
  <tools>cargo check, cargo test</tools>
  <verify>cargo check</verify>
  <reality_test>Start daemon — verify ~/.triumvirate/daemon.pid exists with correct PID. Start second daemon — verify it fails with "Another daemon instance is running." Kill daemon — verify PID file lock is released (flock). Read PID file from external process — verify libproc confirms the PID is the triumvirate binary.</reality_test>
  <done_when>Daemon creates PID file on startup, acquires flock, releases on exit. Stale PID files (crash recovery) are detected and overwritten. Second daemon instance startup is blocked.</done_when>
</task>

<task id="T-006" req="REQ-020" wave="1" depends="T-001">
  <description>Implement event replay ring buffer</description>
  <files>daemon/crates/daemon-core/src/replay.rs, daemon/crates/daemon-core/src/lib.rs</files>
  <scope_out>Do not modify the existing broadcast channel. Add the ring buffer alongside it. Do not change existing WebSocket handler — that's Wave 2.</scope_out>
  <tools>cargo check -p daemon-core, cargo test -p daemon-core</tools>
  <verify>cargo check -p daemon-core</verify>
  <reality_test>Push 1500 events into a 1000-capacity buffer. Request replay from seq 800 — get 200 events (801-1000). Request replay from seq 200 — get OutOfRange (oldest is 501). Request replay from seq 1400 — get 100 events (1401-1500). Verify subscribe-before-read pattern prevents gaps.</reality_test>
  <done_when>EventReplayBuffer with push, replay_since, capacity management. Thread-safe (Arc<RwLock>). Integrated into DaemonState. Tests prove gap-free replay.</done_when>
</task>

<task id="T-007" req="REQ-010" wave="1" depends="T-001">
  <description>Emit WorkerLifecycle events during ABE dispatch and completion</description>
  <files>daemon/crates/triumvirate/src/abe/orchestrator.rs, daemon/crates/triumvirate/src/abe/task_tracker.rs, daemon/crates/triumvirate/src/agent_exec.rs</files>
  <scope_out>Do not modify ABE task execution logic. Only add event emission at lifecycle points. Do not change existing AgentStreamEvent emissions.</scope_out>
  <tools>cargo check, cargo test</tools>
  <verify>cargo check</verify>
  <reality_test>Dispatch a mock worker via ABE. Verify WebSocket receives WorkerLifecycle::Spawned with correct parent_session_id. Complete the worker. Verify WorkerLifecycle::Completed with commit_sha and elapsed_ms. Fail a worker. Verify WorkerLifecycle::Failed with error_message.</reality_test>
  <done_when>WorkerLifecycle events emitted at spawn, completion, and failure. Events carry lineage fields. Events flow through the existing broadcast channel to WebSocket clients.</done_when>
</task>

---

## Wave 2 — Daemon HTTP (v3.9.0)

<task id="T-007.5" req="REQ-017,REQ-020" wave="2" depends="T-006,T-007">
  <description>Wave 2 plumbing prep — wire EventReplayBuffer into ws_events, add TaskTracker.snapshot_workers, add fleet_v2_states to DaemonState. This task was added on 2026-04-11 by Phase 5.3 empirical verification after T-008 and T-009 attempted to read fields that didn't exist. (Phase 4.4 doc audit miss.)</description>
  <files>daemon/crates/daemon-core/src/lib.rs, daemon/crates/triumvirate/src/abe/task_tracker.rs, daemon/crates/triumvirate/src/main.rs, daemon/crates/shared-types/src/api.rs (WorkerSnapshot type)</files>
  <scope_out>Do not modify EventReplayBuffer's public API (T-006 contract). Do not change shared_types::api response shapes. Do not add new HTTP routes — that's T-008/T-009. Do not change McpBridge.fleet_states (legacy MCP fleet path stays untouched).</scope_out>
  <tools>cargo check -p daemon-core -p triumvirate -p shared-types, cargo test -p daemon-core -p triumvirate, file read/write within files list only</tools>
  <verify>cargo check -p daemon-core -p triumvirate -p shared-types</verify>
  <reality_test>(1) Start daemon harness in test. Spawn a TaskTracker with the new constructor. Register two tasks. Call snapshot_workers() — assert it returns 2 entries with correct task_ids and lineage. (2) Construct DaemonState with the new replay_buffer field. Send 5 AgentStreamEvent::ToolCall events through ws_events. Assert daemon_state.replay_buffer.len() == 5 within 100ms (the fill task pushed them). (3) Call DaemonState::new and verify fleet_v2_states starts empty.</reality_test>
  <done_when>DaemonState carries Arc&lt;EventReplayBuffer&gt; and Arc&lt;Mutex&lt;HashMap&lt;String, FleetBuild&gt;&gt;&gt;. A startup hook in run_daemon subscribes to ws_events BEFORE any other subscriber and pushes parsed events into the buffer (subscribe-before-read). TaskTracker::snapshot_workers() returns Vec&lt;WorkerSnapshot&gt; suitable for T-008's WorkerInfo aggregation. Build manifests for T-008 and T-009 reference these new fields by name.</done_when>
</task>

<task id="T-008" req="REQ-017" wave="2" depends="T-002,T-003,T-004,T-007.5">
  <description>Implement GET /api/workers, GET /api/fleet, and GET /api/fleet/{build_id} endpoints (axum 0.8 path syntax — was /:build_id in the pre-audit spec; Phase 5.3 R1 finding fixed the syntax after empirical verification revealed axum 0.8 panics on colon-prefix). /api/workers returns ABE workers only (via TaskTracker::snapshot_workers); named MCP sessions remain on the existing /session/list route.</description>
  <files>daemon/crates/triumvirate/src/main.rs (3 new route handlers + reality tests)</files>
  <scope_out>Do not modify existing HTTP routes. Add new routes only. Do not change auth middleware. Do NOT touch shared_types::api shapes (already finalized in T-002). Do not touch McpBridge, the legacy /api/tokens routes, or /session/list. Do not aggregate SessionState entries into /api/workers — that's a separate future task; T-008 only surfaces ABE workers from TaskTracker::snapshot_workers. Do not introduce new crate dependencies.</scope_out>
  <tools>cargo check -p triumvirate, cargo test -p triumvirate --bin triumvirate, file read/write within files list only</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>(1) Build the Axum router via the same `app = Router::new()...` chain. Tower-test it with a Request to GET /api/workers + bearer token + a TaskTracker that has 2 workers registered with distinct parent_session_ids. Assert response.status() == 200 AND parsed WorkersResponse.workers.len() == 2 AND each entry's parent_session_id matches what was registered. (2) Populate fleet_v2_states (pub field, acquire the lock and insert) with 1 FleetBuild containing 3 tasks. Assert /api/fleet returns it. (3) GET /api/fleet/{build_id} for an existing build returns 200 + that single build; for a missing build returns 404. (4) GET /api/workers without Authorization header returns 401. (5) GET /api/workers with WRONG bearer returns 401. (6) GET /api/workers against an empty TaskTracker returns WorkersResponse{workers:[]} — empty array, NOT null. None of these can pass on a stub that returns hardcoded JSON.</reality_test>
  <done_when>Three GET endpoints registered in run_daemon's Router chain and return correct JSON matching BACKEND_STRUCTURE.md contracts. Auth enforced via the existing per-handler is_bearer_authorized pattern. Empty responses when no workers/fleet active. No regression in existing routes (cargo test --bin triumvirate green). Commit message starts with `T-008:` and references REQ-017.</done_when>
</task>

<task id="T-009" req="REQ-020" wave="2" depends="T-002,T-006,T-007.5">
  <description>Implement GET /api/state endpoint AND a NEW WebSocket route /ws/v2 with the subscribe-with-lastSeq replay handshake. The existing /ws stays untouched for legacy `triumvirate watch` compatibility. Wire format: historical replay frames and live tail frames both use the daemon_core::encode_ws_event("agent_stream", payload) envelope — handshake-response frames (`{"replay":"ok"|"out_of_range",...}`) are bare JSON distinguished by the top-level `"replay"` field.</description>
  <files>daemon/crates/triumvirate/src/main.rs (1 new GET /api/state handler, 1 new WS /ws/v2 route + handshake handler, in-file reality tests)</files>
  <scope_out>Do NOT modify the existing /ws route or daemon-http::ws_route — that's the legacy WebSocket the `triumvirate watch` CLI consumes. Add /ws/v2 alongside it. Do not change EventReplayBuffer's public API. Do not change DaemonState struct fields (T-007.5 owns those). Do not introduce new crate dependencies. Do not modify shared_types::api shapes. StateResponse has NO `sessions` field — the frozen api.rs type carries only version/uptime_ms/workers/fleet/last_event_seq, and MCP sessions remain on the existing /session/list route.</scope_out>
  <tools>cargo check -p triumvirate, cargo test -p triumvirate --bin triumvirate, file read/write within files list only</tools>
  <verify>cargo check -p triumvirate</verify>
  <reality_test>(1) GET /api/state with bearer → assert StateResponse parses with version (String from daemon_core::VERSION.to_string()), uptime_ms (u64 > 0 because state.started_at is in the past), workers (Vec<WorkerInfo>), fleet (Vec<FleetBuild>), last_event_seq (u64). (2) /ws/v2 handshake test using a real ephemeral server (TcpListener::bind("127.0.0.1:0")) and a tokio_tungstenite client: pre-load state.replay_buffer with 5 events seq 1..=5, connect, send {"action":"subscribe","last_seq":0}. Assert client receives ReplayResponse{replay:"ok"}, then 5 envelope-wrapped agent_stream frames in order. (3) Pre-load with seq 1401..=1500 (capacity 100 buffer so eviction pushed older out), send subscribe with last_seq=200. Assert client receives a single bare JSON ReplayResponse{replay:"out_of_range",oldest_seq:1401} (NOT wrapped in envelope) and the connection closes. (4) Pre-load with seq 50..=60, subscribe last_seq=50 → assert client receives events seq 51..=60 wrapped in envelope (10 events, NOT seq=50 itself). (5) Live tail after historical replay: pre-load seq 1..=3, connect, drain ack + 3 historical events, then state.ws_events.send(encode_ws_event("agent_stream", ...seq 4 event...)) → assert client receives the seq=4 frame via the live tail (same envelope shape). (6) Dedup: pre-load seq 1..=5, subscribe last_seq=0, drain 5 historical events, then state.ws_events.send a duplicate envelope for seq=3 → assert client does NOT receive a second seq=3 message (max_sent dedup catches it). (7) Missing bearer on /ws/v2 upgrade → 401 BEFORE protocol switch. (8) Connect to legacy /ws (not /ws/v2) → assert it still emits the 4 hardcoded bootstrap events from daemon_http::ws_route (backwards compat).</reality_test>
  <done_when>GET /api/state returns StateResponse (NO sessions field). /ws/v2 implements the subscribe-with-lastSeq handshake using subscribe-before-read with seq dedup. Historical replay frames and live frames both use the encode_ws_event envelope. Out-of-range surfaced as a bare ReplayResponse JSON object with oldest_seq. Existing /ws unchanged. RecvError::Lagged closes the connection. Commit message starts with `T-009:` and references REQ-020.</done_when>
</task>

<task id="T-010" req="REQ-033" wave="2" depends="T-004">
  <description>ABSORBED INTO T-004 (2026-04-11). Both transport halves (HTTP X-Pantheon-Session-Id middleware + stdio _meta.pantheon.session_id extraction) shipped under T-004 in this session. T-010 is closed without code; the original done_when has been satisfied. See BUILD_MANIFEST.md Wave 1 row.</description>
  <files>(none — see T-004)</files>
  <scope_out>n/a (closed)</scope_out>
  <tools>n/a</tools>
  <verify>cargo test -p triumvirate --bin triumvirate -- pantheon_stdio_meta_tests http_mcp::tests (16/16)</verify>
  <reality_test>16/16 tests pass across pantheon_stdio_meta_tests + http_mcp::tests; both transports proven end-to-end.</reality_test>
  <done_when>ABSORBED INTO T-004 — done.</done_when>
</task>

---

## Wave 3 — Daemon Release (v3.9.0)

<task id="T-011" req="ALL" wave="3" depends="T-008,T-009,T-010">
  <description>Integration tests for all new v3.9.0 daemon features</description>
  <files>daemon/crates/triumvirate/tests/integration_pantheon.rs</files>
  <scope_out>Do not modify existing integration tests. New test file only.</scope_out>
  <tools>cargo test -p triumvirate</tools>
  <verify>cargo test -p triumvirate</verify>
  <reality_test>Full integration test: start daemon, connect via MCP with PANTHEON_SESSION_ID, dispatch ABE worker, verify WebSocket receives WorkerLifecycle events with correct lineage, verify /api/workers returns the worker, verify /api/state returns complete snapshot, disconnect WebSocket, reconnect with lastSeq, verify replay.</reality_test>
  <done_when>All new features tested end-to-end in a single integration test. Existing integration tests still pass.</done_when>
</task>

<task id="T-012" req="ALL" wave="3" depends="T-011">
  <description>Version bump to 3.9.0, CHANGELOG, BUILD_MANIFEST</description>
  <files>daemon/Cargo.toml, CHANGELOG.md, BUILD_MANIFEST.md</files>
  <scope_out>Do not modify source code. Version bump and documentation only.</scope_out>
  <tools>cargo check, git tag</tools>
  <verify>cargo check</verify>
  <reality_test>cargo build --release succeeds. Binary reports version 3.9.0. All tests pass. CHANGELOG has all new features listed.</reality_test>
  <done_when>v3.9.0 tagged, built, tested, documented. Ready for Pantheon to consume.</done_when>
</task>

---

## Wave 4 — Tauri Scaffold (v4.0.0)

<task id="T-013" req="REQ-001,REQ-002,REQ-027,REQ-029,REQ-034" wave="4" depends="T-012">
  <description>Initialize Tauri v2 + Svelte 5 project, configure workspace membership, app shell with three-region layout</description>
  <files>pantheon/ (new directory), pantheon/src-tauri/Cargo.toml, pantheon/src-tauri/src/main.rs, pantheon/src-tauri/src/lib.rs, pantheon/src-tauri/tauri.conf.json, pantheon/src-tauri/capabilities/default.json, pantheon/package.json, pantheon/svelte.config.js, pantheon/vite.config.ts, pantheon/src/App.svelte, pantheon/src/app.css, daemon/Cargo.toml (add workspace member)</files>
  <scope_out>Do not build terminal panels, sidebar content, or status area content yet. Shell layout only. Do not modify daemon source code.</scope_out>
  <tools>npm run tauri dev, cargo check</tools>
  <verify>npm run tauri dev launches successfully, window appears with three empty regions</verify>
  <reality_test>Launch the app. Verify: three-region layout renders (sidebar 250px, terminal area flex:1, status area 280px). Window title shows "Pantheon". Minimum size 900x600 enforced. Cmd+B toggles sidebar. Cmd+Shift+B toggles status area. Status area auto-collapses below 1200px width.</reality_test>
  <done_when>Tauri v2 app launches, shows three-region flexbox layout with collapsible sidebar and status area, workspace membership in daemon/Cargo.toml confirmed, shared-types accessible via path dependency.</done_when>
</task>

<task id="T-014" req="REQ-003,REQ-019" wave="4" depends="T-013">
  <description>Menubar tray icon with 4-state template images and close-to-tray behavior</description>
  <files>pantheon/src-tauri/src/tray.rs, pantheon/src-tauri/src/lib.rs, pantheon/src-tauri/icons/ (4 template PNGs)</files>
  <scope_out>Do not implement daemon connection yet. Use mock state for tray icon testing. Do not implement window content beyond the shell.</scope_out>
  <tools>npm run tauri dev</tools>
  <verify>App shows tray icon in menubar</verify>
  <reality_test>Launch app. Verify menubar icon appears (filled circle). Close window (Cmd+W) — verify window hides, app stays running, tray icon visible. Click tray icon — verify window reopens and focuses. Right-click tray icon — verify dropdown menu with "Quit" option. Programmatically set each state (starting/ready/degraded/disconnected) — verify icon changes shape.</reality_test>
  <done_when>Four template image icons (22x22 @2x), close-to-tray pattern working, left-click reopens window, right-click shows menu, all states render correctly in both dark and light mode.</done_when>
</task>

<task id="T-015" req="REQ-022,REQ-031" wave="4" depends="T-013">
  <description>Custom native menu (Edit without Find), dark mode detection, Tailwind setup</description>
  <files>pantheon/src-tauri/src/menu.rs, pantheon/src/lib/stores/preferences.ts, pantheon/tailwind.config.ts, pantheon/src/app.css</files>
  <scope_out>Do not implement terminal panels. Dark mode applies to app shell only at this stage.</scope_out>
  <tools>npm run tauri dev</tools>
  <verify>App launches without white flash, respects system dark/light mode</verify>
  <reality_test>Launch in dark mode — verify dark background, no white flash. Switch system to light mode — verify app updates without reload. Open Edit menu — verify "Find" is NOT present. Press Cmd+F — verify no WKWebView native search appears. Toggle between Light/Dark/Auto in (mock) preferences UI.</reality_test>
  <done_when>Dark mode auto-detection working with Svelte 5 $effect. No white flash. Edit menu customized (no Find). Tailwind dark: classes functional. tauri-plugin-prevent-default suppressing browser shortcuts.</done_when>
</task>

---

## Wave 5 — Terminal Panels (v4.0.0)

<task id="T-016" req="REQ-004,REQ-007" wave="5" depends="T-013">
  <description>xterm.js + PTY integration — spawn Claude Code, render in terminal panel</description>
  <files>pantheon/src/lib/components/TerminalPanel.svelte, pantheon/src-tauri/src/pty.rs, pantheon/src-tauri/src/lib.rs, pantheon/package.json</files>
  <scope_out>Do not implement tabs or splits yet. Single terminal panel only. Do not implement sidebar or status area content.</scope_out>
  <tools>npm run tauri dev, cargo check</tools>
  <verify>npm run tauri dev, terminal panel shows Claude Code greeting</verify>
  <reality_test>Launch app. Trigger terminal creation (hardcoded to ~/projects/triumvirate for testing). Verify: Claude Code starts, greeting renders with full formatting (colors, markdown, status line). Type a message — verify it appears in Claude. Claude responds — verify streaming text renders at 60fps. Resize window — verify terminal reflows correctly. Scroll up — verify scrollback works (5000 lines). Press Cmd+F — verify search bar appears (addon-search).</reality_test>
  <done_when>Claude Code runs in xterm.js terminal panel with full formatting. PTY bidirectional. WebGL rendering. Scrollback. Search. Resize handling. No escape sequence artifacts.</done_when>
</task>

<task id="T-017" req="REQ-005,REQ-006,REQ-008,REQ-032" wave="5" depends="T-016">
  <description>Tab bar, directory picker, splits (PaneForge), session ended UI, recent projects</description>
  <files>pantheon/src/lib/components/TabBar.svelte, pantheon/src/lib/components/TerminalArea.svelte, pantheon/src/lib/stores/sessions.ts</files>
  <scope_out>Do not implement sidebar worker hierarchy. Terminal panel management only.</scope_out>
  <tools>npm run tauri dev</tools>
  <verify>Cmd+T opens directory picker, new tab appears, Cmd+D splits</verify>
  <reality_test>Press Cmd+T — directory picker or recent projects dropdown appears. Select a project — new tab with Claude Code appears. Press Cmd+T again — second tab. Switch tabs with Cmd+1, Cmd+2. Press Cmd+D — active tab splits horizontally, new Claude Code instance in split. Close Claude Code (type /exit) — "Session ended" with Restart/Close buttons. Press Restart — new Claude Code in same directory. Recent projects persisted across app restarts (tauri-plugin-store).</reality_test>
  <done_when>Full terminal panel lifecycle: create (Cmd+T with directory picker + recents), tabs, splits (PaneForge), switch (Cmd+1-9), close (Cmd+W), session ended UI, persistence.</done_when>
</task>

<task id="T-018" req="REQ-023,REQ-035" wave="5" depends="T-017">
  <description>Quit confirmation dialog and background notification for PTY input</description>
  <files>pantheon/src-tauri/src/lifecycle.rs, pantheon/src-tauri/src/lib.rs</files>
  <scope_out>Do not implement daemon connection. Quit and notification logic only.</scope_out>
  <tools>npm run tauri dev</tools>
  <verify>Cmd+Q with active session shows confirmation dialog</verify>
  <reality_test>Open a terminal panel. Press Cmd+Q — verify confirmation "1 session still active. Quit anyway?" Cancel — app stays. Quit — Claude Code gets SIGTERM, app exits. Close window (Cmd+W) with active Claude session. Wait for Claude to prompt for input. Verify macOS notification appears within 10 seconds. Click notification — window reopens.</reality_test>
  <done_when>Quit confirmation when active sessions exist. SIGTERM to PTY children on quit. Background notifications (best-effort) when Claude appears to wait for input. Debounced 1/min/session.</done_when>
</task>

<task id="T-019" req="REQ-033" wave="5" depends="T-016">
  <description>Set PANTHEON_SESSION_ID env var on PTY child processes</description>
  <files>pantheon/src-tauri/src/pty.rs</files>
  <scope_out>Do not implement daemon-side linking (that's v3.9.0). Only set the env var on spawn.</scope_out>
  <tools>cargo check, npm run tauri dev</tools>
  <verify>cargo check</verify>
  <reality_test>Launch app, create terminal panel. In Claude Code, run: echo $PANTHEON_SESSION_ID. Verify it outputs a UUID. Create a second terminal panel. Verify different UUID. Verify the MCP proxy inherits the env var (check Claude Code's MCP server process environment).</reality_test>
  <done_when>Each terminal panel's Claude Code process has a unique PANTHEON_SESSION_ID env var that propagates to child processes including the MCP proxy.</done_when>
</task>

---

## Wave 6 — Sidebar + Status Area (v4.0.0)

<task id="T-020" req="REQ-019,REQ-020,REQ-021" wave="6" depends="T-013">
  <description>Daemon client — WebSocket connection, REST polling, health state machine</description>
  <files>pantheon/src-tauri/src/daemon_client.rs, pantheon/src/lib/stores/daemon.ts, pantheon/src-tauri/src/lib.rs</files>
  <scope_out>Do not implement sidebar UI. Backend client and Svelte stores only.</scope_out>
  <tools>cargo check, npm run tauri dev (with daemon running)</tools>
  <verify>App connects to daemon, Svelte store populated with worker data</verify>
  <reality_test>Start daemon (v3.9.0). Launch Pantheon. Verify: WebSocket connects, daemon state = "ready", menubar icon = filled circle. Kill daemon — verify state transitions to "disconnected" within 2 seconds, menubar icon changes. Restart daemon — verify reconnect within 10 seconds, state = "ready". Check Svelte store — verify workers array populated from /api/workers. Verify /api/tokens data flows into store.</reality_test>
  <done_when>DaemonClient connects via WebSocket, polls REST endpoints, manages 4-state health machine, forwards events to Svelte stores via Tauri events. Reconnect with seq-based replay or /api/state fallback.</done_when>
</task>

<task id="T-021" req="REQ-009,REQ-010,REQ-011,REQ-012" wave="6" depends="T-020,T-019">
  <description>Sidebar UI — hierarchical session/worker tree, worker detail drawer</description>
  <files>pantheon/src/lib/components/Sidebar.svelte, pantheon/src/lib/components/WorkerDrawer.svelte, pantheon/src/lib/stores/workers.ts</files>
  <scope_out>Do not implement unmanaged session scanning. Daemon-managed sessions and workers only.</scope_out>
  <tools>npm run tauri dev (with daemon running)</tools>
  <verify>Sidebar shows terminal panels as top-level nodes with daemon workers nested underneath</verify>
  <reality_test>Create terminal panel in Pantheon. Tell Claude to dispatch a Codex worker. Verify: worker appears as indented child under the terminal panel's node within 1 second. Worker shows status (queued → working → committed). Click worker — detail drawer opens with event stream. Worker completes — entry dims with checkmark. Dispatch 6 workers — verify all appear under the correct parent. Fail a worker — verify red indicator on worker AND parent node (status bubbling).</reality_test>
  <done_when>Hierarchical sidebar with sessions → workers. Workers auto-appear from WebSocket events. Detail drawer shows filtered event stream. Status bubbling. 3-level cap. "Daemon Workers" section for unlinked workers.</done_when>
</task>

<task id="T-022" req="REQ-015,REQ-016,REQ-017" wave="6" depends="T-020">
  <description>Status area — Token Economics, Fleet Status panels</description>
  <files>pantheon/src/lib/components/StatusArea.svelte, pantheon/src/lib/components/TokenEconomics.svelte, pantheon/src/lib/components/FleetStatus.svelte</files>
  <scope_out>Do not implement System Health (that's Wave 7 with process scanner). Token and Fleet panels only.</scope_out>
  <tools>npm run tauri dev (with daemon running)</tools>
  <verify>Status area shows token volumes and fleet task statuses</verify>
  <reality_test>Run a fleet build via Claude. Verify: Token Economics shows "248K in / 31K out / 71% cached" (real values from daemon). Fleet Status shows T-001 through T-006 with live status updates. Completed tasks show checkmark. Sections are collapsible.</reality_test>
  <done_when>Token Economics panel showing per-session and aggregate token volume. Fleet Status panel showing ABE tasks with real-time status. Both consuming daemon REST + WebSocket data. Collapsible sections.</done_when>
</task>

<task id="T-023" req="REQ-013,REQ-014,REQ-018" wave="6" depends="T-013">
  <description>Process scanner and System Health panel — unmanaged sessions, memory pressure</description>
  <files>pantheon/src-tauri/src/process_scanner.rs, pantheon/src/lib/components/SystemHealth.svelte, pantheon/src/lib/components/UnmanagedSessions.svelte</files>
  <scope_out>Do not modify daemon. Client-side process scanning only.</scope_out>
  <tools>npm run tauri dev</tools>
  <verify>Unmanaged Claude sessions appear in sidebar, System Health shows memory pressure</verify>
  <reality_test>Open 2 Claude Code sessions in Terminal.app (outside Pantheon). Launch Pantheon. Verify: both appear in Unmanaged section within 5 seconds with PID, project directory, Physical Footprint memory, idle time. Verify warning badge on sessions idle >30 min. Click Kill — confirmation dialog — verify process dies and entry removed on next scan. System Health shows total agent memory and pressure bar. Verify graceful degradation if macOS blocks process introspection.</reality_test>
  <done_when>Process scanner finds unmanaged claude/gemini/codex processes via full cmdline matching. Physical Footprint memory (not RSS). Kill with confirmation. System Health panel with memory pressure bar. Graceful degradation.</done_when>
</task>

---

## Wave 7 — Polish (v4.0.0)

<task id="T-024" req="REQ-031,REQ-032" wave="7" depends="T-015,T-017">
  <description>Complete dark mode + persistence — theme in xterm.js, all preferences, layout state</description>
  <files>pantheon/src/lib/stores/preferences.ts, pantheon/src/lib/components/TerminalPanel.svelte</files>
  <scope_out>Do not add new features. Polish existing theme and persistence implementations.</scope_out>
  <tools>npm run tauri dev</tools>
  <verify>Theme persists across restarts, xterm.js matches system theme</verify>
  <reality_test>Set theme to Dark. Quit and relaunch — verify dark mode persists. Switch system to Light — verify xterm.js terminal backgrounds change to light. Resize window, move it, relaunch — verify position/size restored. Change sidebar width, relaunch — verify PaneForge layout restored.</reality_test>
  <done_when>All preferences persisted via tauri-plugin-store. Window state via tauri-plugin-window-state. xterm.js theme synced with system/user preference. Layout persistence via PaneForge.</done_when>
</task>

<task id="T-025" req="REQ-030" wave="7" depends="T-014">
  <description>CLI command (pantheon) and deep-link URL scheme (pantheon://)</description>
  <files>pantheon/src-tauri/src/lib.rs, pantheon/src-tauri/tauri.conf.json, scripts/install-pantheon.sh</files>
  <scope_out>Do not modify daemon install script. New Pantheon-specific install script.</scope_out>
  <tools>npm run tauri dev</tools>
  <verify>Running `open pantheon://` from Terminal focuses the app</verify>
  <reality_test>Build the app. Run Pantheon. Open Terminal.app. Run `open pantheon://` — verify Pantheon window focuses. Run `open -a Pantheon` — same result. Install the CLI symlink. Run `pantheon` from Terminal — verify app opens/focuses. Run `pantheon` when not running — verify app launches.</reality_test>
  <done_when>tauri-plugin-single-instance prevents duplicates. tauri-plugin-deep-link handles pantheon:// URLs. CLI symlink at /usr/local/bin/pantheon. Install script creates symlink.</done_when>
</task>

<task id="T-026" req="REQ-019" wave="7" depends="T-020">
  <description>Daemon auto-start as detached process with startup self-check</description>
  <files>pantheon/src-tauri/src/daemon_client.rs</files>
  <scope_out>Do not modify daemon binary. Pantheon spawns existing binary as detached process.</scope_out>
  <tools>npm run tauri dev (with daemon NOT running)</tools>
  <verify>Launch Pantheon with no daemon running — daemon auto-starts</verify>
  <reality_test>Ensure daemon is stopped. Launch Pantheon. Verify: menubar icon shows pulsing (starting). Within 3 seconds: daemon process appears in ps, /health returns 200, WebSocket connects, icon changes to filled circle (ready). Quit Pantheon — verify daemon stays running (detached). Relaunch Pantheon — verify connects to existing daemon without spawning another.</reality_test>
  <done_when>Daemon auto-starts as detached process if not running. PID file checked first. Health poll until ready. Version handshake (warn on mismatch). Daemon survives Pantheon quit.</done_when>
</task>

<task id="T-027" req="REQ-024,REQ-025,REQ-026" wave="7" depends="T-016,T-021,T-023">
  <description>Performance profiling and memory budget verification</description>
  <files>pantheon/PERFORMANCE_REPORT.md</files>
  <scope_out>Do not change features. Measure, optimize if needed, document results.</scope_out>
  <tools>npm run tauri dev, Activity Monitor, Instruments</tools>
  <verify>App shell <100MB, per panel ~60MB, launch <3s, 60fps rendering</verify>
  <reality_test>Launch Pantheon with 0 terminal panels. Measure Physical Footprint — must be <100MB. Open 1 terminal panel — measure delta (~60MB). Open 5 terminal panels — measure total (~400MB). Switch to background tab — verify WebGL disposed (memory drops). Stream large output — verify 60fps in Instruments. Measure launch time — must be <3 seconds to usable state.</reality_test>
  <done_when>Performance report documenting measured values for all budgets. All targets met or documented deviations with remediation plan.</done_when>
</task>

---

## Wave 8 — Build + Release (v4.0.0)

<task id="T-028" req="REQ-028,REQ-029" wave="8" depends="T-027">
  <description>Build unsigned .dmg for macOS aarch64</description>
  <files>pantheon/src-tauri/tauri.conf.json (bundle config), pantheon/scripts/build.sh</files>
  <scope_out>Do not implement code signing. Unsigned only.</scope_out>
  <tools>npm run tauri build</tools>
  <verify>.dmg file produced, installs to /Applications, launches</verify>
  <reality_test>Run npm run tauri build. Verify .dmg file exists in target/release/bundle/dmg/. Mount .dmg. Drag to Applications. Launch from Applications. Right-click → Open on first launch (Gatekeeper bypass). Verify app works: terminal panels, sidebar, status area, menubar, daemon connection. Verify daemon binary is bundled inside the .app.</reality_test>
  <done_when>Unsigned .dmg builds for aarch64. App installs and runs. Daemon bundled. All features functional from installed .app.</done_when>
</task>

<task id="T-029" req="ALL" wave="8" depends="T-028">
  <description>Version bump to 4.0.0, CHANGELOG, BUILD_MANIFEST, README update</description>
  <files>daemon/Cargo.toml, CHANGELOG.md, BUILD_MANIFEST.md, README.md, ROADMAP.md</files>
  <scope_out>Documentation only. No code changes.</scope_out>
  <tools>git tag</tools>
  <verify>All docs updated, tag created</verify>
  <reality_test>CHANGELOG lists all v3.9.0 and v4.0.0 features. README references Pantheon. ROADMAP updated with v4.0 shipped. BUILD_MANIFEST has entries for every task. git tag v4.0.0 created.</reality_test>
  <done_when>v4.0.0 tagged, documented, manifested. Pantheon is shipped.</done_when>
</task>

---

## Execution Contract

### Backlog Freeze
This document contains 29 tasks across 9 waves. This is the COMPLETE backlog.
- Do NOT accept new tasks until all tasks are complete (backlog_status: 0).
- If new requirements arrive mid-execution, respond: `blocked_on: scope-change — [describe new requirement]` and STOP.
- Only the human can add, remove, or reorder tasks in this backlog.

### Execution Order
- Wave order is strict: complete ALL tasks in Wave N before starting Wave N+1.
- Within a wave: tasks WITHOUT `depends` on each other are parallel-safe. Tasks WITH intra-wave `depends` must execute in dependency order (the `depends` attribute is the sequencing mechanism). Example: if T-017 depends on T-016 and both are in Wave 5, T-016 must complete before T-017 starts, but T-019 (which only depends on T-016) can run in parallel with T-017.
- Within a sequential group: strict FIFO based on `depends` chain.

### Definition of Done (Per Task)
A task is DONE when ALL of these are true:
1. Code is written (not stubbed — see reality test)
2. `<verify>` passes (compilation/type check)
3. `<reality_test>` passes (behavioral check that a stub cannot fake)
4. `<done_when>` condition is met (semantic completion check)
5. FULL test suite passes (`cargo test` for Rust, `npm test` for frontend) — not just this task's tests
6. Git commit is created with message referencing task ID

A task that passes its own tests but breaks other tests is NOT done. Fix the regression first.

### Commit Report Format
After each task commit, respond with EXACTLY this format and nothing else:
```
task: T-{ID}
commit: {hash}
changed: {1-5 bullets, one per file or logical change}
tests: {exact command} → {pass count}/{total count} passed
remaining: {N} tasks in current wave, {M} total
```

### Collateral Fix Protocol
If completing a task REQUIRES touching files outside that task's `<files>` list:
1. Label the commit: `collateral-fix: T-{ID} — {one-line justification}`
2. List extra files in the commit report under a `collateral:` field
3. Re-run full test suite after the collateral fix

### Blocked Protocol
If blocked on any task, respond with EXACTLY:
```
blocked_on: {single concrete blocker}
task: T-{ID}
evidence: {command + output summary, max 5 lines}
proposed_fix: {single action you would take}
```

### Self-Validation (MANDATORY)
After each task commit, run the validation script:
```
~/.claude/scripts/validate-task.sh T-{ID} "{test command}" {files from <files> list}
```

### End-of-Execution Report
When all tasks are complete, respond with:
```
backlog_status: 0 remaining
completed_tasks: [T-001, T-002, ...]
total_commits: {N}
collateral_fixes: {N} ({list if any})
validation: {N}/{N} tasks passed validate-task.sh
test_suite: {exact command} → {pass/fail with counts}
```
