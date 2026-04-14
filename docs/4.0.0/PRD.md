# Pantheon v4.0 — Product Requirements Document

**Version:** 4.0.0  
**Spec:** specs/PANTHEON_V4.md  
**Release Strategy:** v3.9.0 (daemon backend) → v4.0.0 (Tauri app)  

---

## Product Overview

Pantheon is a native macOS application for multi-agent orchestration. It replaces 17+ Terminal.app windows with one GUI that shows embedded Claude Code sessions, daemon-spawned workers in a hierarchical sidebar, system memory pressure, and fleet build status. Built with Tauri v2 + Svelte 5 + xterm.js.

---

## Features

### FEAT-001: Embedded Terminal Panels (REQ-004, REQ-005, REQ-006, REQ-007, REQ-008)

Claude Code runs inside xterm.js terminal panels within the Pantheon window. Full formatting, colors, markdown, tool approval UI, and slash commands work unchanged.

**Acceptance Criteria:**
- AC-001.1: User presses Cmd+T → directory picker → Claude Code starts in xterm.js panel
- AC-001.2: Claude Code renders identically to Terminal.app (colors, markdown, status line)
- AC-001.3: Shift+Enter produces multiline input (Kitty keyboard protocol)
- AC-001.4: Terminal panels support tabs and horizontal/vertical splits (PaneForge)
- AC-001.5: Scrollback of 5,000 lines with Cmd+F search (addon-search)
- AC-001.6: WebGL rendering at 60fps during streaming
- AC-001.7: Session ended → "Restart" or "Close" buttons shown
- AC-001.8: Recent projects dropdown (max 10) on Cmd+T

### FEAT-002: Hierarchical Worker Sidebar (REQ-009, REQ-010, REQ-011, REQ-012)

Workers appear nested under the Claude session that dispatched them. Hierarchy shows session → worker → detail drawer.

**Acceptance Criteria:**
- AC-002.1: Sidebar shows tree: session name + project path at top level, workers indented below
- AC-002.2: Workers appear within 1 second of daemon dispatch
- AC-002.3: Worker entries show: agent icon, task ID, target files, status, elapsed time
- AC-002.4: Clicking a worker opens a detail drawer (bottom/right panel) with event stream
- AC-002.5: Completed workers dimmed with checkmark; failed workers red
- AC-002.6: Status bubbles: child failure → parent turns red
- AC-002.7: Hierarchy capped at 3 visible levels
- AC-002.8: Workers dispatched without PANTHEON_SESSION_ID appear under "Daemon Workers"

### FEAT-003: Unmanaged Session Scanner (REQ-013, REQ-014)

Discovers Claude/Gemini/Codex processes running outside Pantheon and the daemon.

**Acceptance Criteria:**
- AC-003.1: Process scan every 5 seconds via libproc/sysinfo
- AC-003.2: Shows PID, project directory (cwd via PROC_PIDVNODEPATHINFO), Physical Footprint memory, idle time
- AC-003.3: Full command-line argument matching (not just executable name)
- AC-003.4: Warning badge if idle >30 min or memory >300MB
- AC-003.5: Kill button with confirmation dialog
- AC-003.6: Graceful degradation if macOS blocks process introspection

### FEAT-004: System Health Dashboard (REQ-015, REQ-016, REQ-017, REQ-018)

Token economics, fleet status, and memory pressure in the status area.

**Acceptance Criteria:**
- AC-004.1: Token volume display: "248K in / 31K out / 71% cached" per session
- AC-004.2: Aggregate display: "Today: 1.2M in / 180K out"
- AC-004.3: Fleet status: task ID, files, status, elapsed time — real-time updates
- AC-004.4: Memory pressure bar with Physical Footprint (not RSS)
- AC-004.5: Red bar + notification when agent memory >60% of system RAM
- AC-004.6: Three collapsible sections in status area

### FEAT-005: Daemon Connection Management (REQ-019, REQ-020, REQ-021)

Four-state health machine with auto-start, reconnect, and event replay.

**Acceptance Criteria:**
- AC-005.1: Four states: starting (pulsing icon) → ready (filled circle) → degraded (exclamation) → disconnected (slash)
- AC-005.2: Auto-starts daemon as detached process if not running
- AC-005.3: Daemon token read from ~/.triumvirate/daemon.token
- AC-005.4: WebSocket reconnect with exponential backoff, max 10 seconds
- AC-005.5: Seq-based replay for short gaps (<30s), full /api/state refresh for long gaps
- AC-005.6: Terminal panels functional during all daemon states
- AC-005.7: Startup self-check: verify daemon reachable, warn if not

### FEAT-006: Session Linking (REQ-033)

Links Pantheon terminal panels to daemon sessions for hierarchical worker attribution.

**Acceptance Criteria:**
- AC-006.1: PANTHEON_SESSION_ID env var set on each Claude Code child process
- AC-006.2: MCP proxy sends it via X-Pantheon-Session-Id HTTP header
- AC-006.3: MCP stdio transport sends it via _meta in initialize request
- AC-006.4: Daemon open_session handshake returns canonical session_id
- AC-006.5: Lineage (parent_session_id, root_session_id) persisted in SQLite
- AC-006.6: Missing PANTHEON_SESSION_ID → workers appear under "Daemon Workers"

### FEAT-007: Native macOS Integration (REQ-001, REQ-002, REQ-003, REQ-022, REQ-023)

App shell, menubar, keyboard shortcuts, quit behavior.

**Acceptance Criteria:**
- AC-007.1: Launch from Spotlight, Dock, or `pantheon` CLI command
- AC-007.2: Three-region layout: sidebar (collapsible), terminal area, status area (collapsible)
- AC-007.3: Menubar template icons (shape-based, dark/light auto-adapt)
- AC-007.4: Standard macOS shortcuts (Cmd+T, Cmd+W, Cmd+D, Cmd+B, Cmd+F, Cmd+Q)
- AC-007.5: Cmd+Q shows confirmation if active sessions exist
- AC-007.6: Cmd+W hides window, app stays in menubar
- AC-007.7: Left-click menubar icon reopens window
- AC-007.8: Single instance via tauri-plugin-single-instance + deep-link
- AC-007.9: Window 900x600 minimum, status area auto-collapses <1200px

### FEAT-008: Dark Mode and Persistence (REQ-031, REQ-032)

System theme detection, user preferences, layout persistence.

**Acceptance Criteria:**
- AC-008.1: Auto-detect system dark/light mode
- AC-008.2: Three modes: Light, Dark, Auto
- AC-008.3: xterm.js theme updates on system theme change
- AC-008.4: No white flash on startup (window starts hidden, shown after theme loads)
- AC-008.5: Settings stored in ~/Library/Application Support/ via tauri-plugin-store
- AC-008.6: Window position/size auto-restored via tauri-plugin-window-state
- AC-008.7: Recent projects list persisted (max 10)

### FEAT-009: Background Behavior (REQ-035)

PTY processes continue when window is hidden. Best-effort notifications.

**Acceptance Criteria:**
- AC-009.1: PTY processes run while window hidden
- AC-009.2: Notification when Claude appears to wait for input (idle >5s + prompt pattern)
- AC-009.3: Notifications debounced (1 per session per minute)
- AC-009.4: Notification click reopens and focuses window
- AC-009.5: User can disable notifications in preferences

### FEAT-010: Build and Distribution (REQ-027, REQ-028, REQ-029, REQ-030, REQ-034)

### FEAT-016: Performance Budgets (REQ-024, REQ-025, REQ-026)

Memory, launch time, and rendering performance constraints.

**Acceptance Criteria:**
- AC-016.1: App shell < 100MB Physical Footprint
- AC-016.2: Per terminal panel ~60MB (WebGL disposed on hidden tabs)
- AC-016.3: 5 panels total ~400MB
- AC-016.4: App launch < 3 seconds to usable state on M4 MacBook Pro
- AC-016.5: 60fps terminal rendering during Claude streaming (frame time <16.7ms)
- AC-016.6: All budgets verified via Instruments profiling gates

---

**Note:** REQ-034 (CSS flexbox outer layout + PaneForge inner) is covered by FEAT-007 and FEAT-010.

### FEAT-010 (continued): Build and Distribution

Tauri v2 build, unsigned .dmg, Apple Silicon only.

**Acceptance Criteria:**
- AC-010.1: Compiles for macOS aarch64
- AC-010.2: Distributed as unsigned .dmg
- AC-010.3: Daemon binary bundled in .app bundle
- AC-010.4: `pantheon` CLI symlink at /usr/local/bin/pantheon
- AC-010.5: pantheon:// URL scheme registered
- AC-010.6: src-tauri in daemon Cargo workspace, shared-types via path dependency

---

## v3.9.0 Daemon Features (Backend prerequisite)

### FEAT-011: Worker Lineage Tracking

**Acceptance Criteria:**
- AC-011.1: parent_session_id + root_session_id on every worker record
- AC-011.2: Captured from MCP caller's session context during dispatch
- AC-011.3: Persisted in SQLite, survives daemon restarts
- AC-011.4: PANTHEON_SESSION_ID captured from _meta and HTTP headers

### FEAT-012: Pantheon REST API Surface

**Acceptance Criteria:**
- AC-012.1: GET /api/workers — all active sessions/workers with lineage and status
- AC-012.2: GET /api/fleet/* — ABE task status per task
- AC-012.3: GET /api/state — full state snapshot (sessions, workers, hierarchy, fleet)
- AC-012.4: All endpoints require bearer token from daemon.token

### FEAT-013: Event Replay

**Acceptance Criteria:**
- AC-013.1: Ring buffer of last 1,000 AgentStreamEvents in memory
- AC-013.2: Client sends lastSeenSeq on WebSocket connect
- AC-013.3: Daemon replays events with seq > lastSeenSeq
- AC-013.4: Out-of-range → returns {"replay": "out_of_range"}
- AC-013.5: Subscribe acquired before reading buffer (no gap)

### FEAT-014: WorkerLifecycle Events

**Acceptance Criteria:**
- AC-014.1: New AgentStreamEvent variant: WorkerLifecycle
- AC-014.2: Sub-types: spawned, completed, failed
- AC-014.3: Carries parent_session_id, root_session_id, task_id

### FEAT-015: PID File Management

**Acceptance Criteria:**
- AC-015.1: PID file at ~/.triumvirate/daemon.pid
- AC-015.2: flock locking via pidfile-rs
- AC-015.3: Atomic write/rename
- AC-015.4: Verify PID matches triumvirate binary via libproc before kill
