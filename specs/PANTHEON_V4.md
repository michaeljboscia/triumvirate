# Pantheon v4.0 — Native Mac App for Multi-Agent Orchestration

**Status:** SPEC FINAL — goatrodeo Phase 3 passed, 4 twin-validated rounds  
**Branch:** v3.9.0 (daemon backend) → v4.0.0 (Tauri app)  
**Working Directory:** /Users/you/projects/triumvirate  
**Author:** Mike Boscia  
**Date:** 2026-04-11  

---

## The Rename

Triumvirate becomes **Pantheon** at v4.0. A triumvirate is three — a pantheon is all the gods. The product is not limited to three agents anymore.

The daemon binary stays `triumvirate` for backwards compatibility until v5.0. The app is called **Pantheon**.

---

## Problem Statement

The user runs 17 Claude Code sessions in separate Terminal.app windows. There is no unified view of what agents are doing. Daemon-spawned workers (Codex worktrees, Gemini sessions, sub-agents) are invisible — the user has no way to see them without opening more terminals. The M4 MacBook runs out of 36GB RAM from agent process proliferation. Terminal-based solutions (TUIs, tmux wrappers) create a terminal-in-terminal experience the user rejects.

**Prior art tried and rejected:**
- Ratatui TUI — terminal-in-terminal, breaks Claude Code's React+Ink rendering
- aoe (Agent of Empires) — tmux wrapper, manual session management, can't see daemon-spawned workers, arcane keybindings
- Zellij wrapping (v3.1.0 retro) — same terminal-in-terminal problem, raw panes not structured status

**What works:** Cursor-style native GUI apps with embedded terminal panels. xterm.js renders Claude Code correctly. No escape sequence conflicts. No keybinding clashes.

---

## Constitution

These principles never change:

1. Claude is the front door. The user talks to Claude.
2. Lifecycle is always visible. No silent failures.
3. Plain language in, structured results out. No command ceremony.
4. Failure is loud, immediate, and actionable.

---

## Requirements

### App Shell

**REQ-001:** Pantheon is a native macOS application built with Tauri v2. The user launches it from Spotlight, the Dock, or the command line (`pantheon`). The app opens a main window within 3 seconds.

**REQ-002:** The main window has a three-region layout: a sidebar (left), a terminal area (center), and a status area (right). The sidebar and status area can be collapsed with a click or keyboard shortcut.

**REQ-003:** The app shows a menubar icon (system tray). The icon uses macOS template images (black + transparent PNG, 22x22 @2x) that auto-adapt to dark/light mode. Four icon variants convey daemon state through shape, not color: filled circle (ready), circle with exclamation (degraded), circle with slash (disconnected), pulsing/animated (starting). Clicking the icon shows a dropdown with: session count, active worker count, total memory usage, and "Open Pantheon" button. Left-click opens/focuses the main window. Right-click shows the dropdown menu. Closing the main window (Cmd+W) hides it — Pantheon stays running in the menubar. `set_icon_as_template(true)` is called after every icon change for Liquid Glass compatibility.

### Terminal Panels

**REQ-004:** The center area contains one or more terminal panels. Each panel is a full xterm.js terminal emulator. Claude Code runs inside these panels with all formatting, colors, markdown rendering, tool approval UI, and slash commands intact.

**REQ-005:** The user creates a new terminal panel via Cmd+T or a "+" button. The new panel spawns Claude Code in a specified project directory. The user is prompted to pick a project directory on first creation.

**REQ-006:** Terminal panels can be arranged as tabs (one visible at a time, tab bar at top) or split horizontally/vertically (multiple visible). The user switches arrangement via Cmd+D (split) or Cmd+W (close panel). Tabs show the project directory name.

**REQ-007:** Each terminal panel is a real PTY. Claude Code is spawned as a child process with a pseudo-terminal via `portable-pty` (or `tauri-plugin-pty`). All stdin/stdout/stderr flow through the PTY. The terminal panel supports resize (coordinated between xterm.js FitAddon and PTY master resize), scroll-back (5,000 lines — reduced from 10,000 for memory), copy/paste, text selection, and in-terminal search via `@xterm/addon-search` (Cmd+F triggers search bar, not WKWebView's native find-in-page — achieved by omitting "Find" from the native Edit menu). xterm.js uses WebGL addon (`@xterm/addon-webgl`) for 60fps rendering, with `webglcontextlost` event handler for recovery when macOS aggressively reclaims contexts. The PTY slave fd is dropped immediately after spawn to ensure EOF detection when Claude Code exits or crashes.

**REQ-008:** When a terminal panel's Claude Code process exits, the panel shows "Session ended" and offers "Restart" or "Close" buttons. The panel does not auto-close.

### Sidebar — Sessions and Workers

**REQ-009:** The sidebar shows a hierarchical tree of sessions and their children. Each terminal panel the user created is a top-level node showing the project name and directory. Daemon-spawned workers (Codex worktrees, Gemini sessions, Claude sub-agents) appear as **indented children** underneath the session that spawned them. This makes it immediately obvious which workers belong to which project and which Claude session dispatched them.

```
▼ triumvirate  ~/projects/triumvirate  active
    codex  T-001  src/auth.rs    committed ✓  42s
    codex  T-002  src/db.rs      working      1m03s
    codex  T-003  src/api.rs     failed ✗     1m12s
    gemini review                 idle         45s
▼ gtm-machine  ~/projects/gtm-machine  active
    codex  T-010  src/scanner.rs working      12s
  Unmanaged
    PID 40877  claude  ~/projects/tellus  485MB  idle 3h ⚠
```

**REQ-010:** Worker entries auto-populate by subscribing to the daemon's `/ws` WebSocket endpoint. When the daemon dispatches a Codex worker, Gemini session, or Claude sub-agent, it appears as a child of the originating session within 1 second. The daemon tracks lineage via two new fields on every worker record: `parent_session_id` (the immediate session that triggered the dispatch, captured from the MCP caller's session context) and `root_session_id` (the top-level user session, for deeply nested dispatch chains). Both fields are persisted in the daemon's SQLite store so the hierarchy survives daemon restarts. When a worker completes, its entry stays visible (dimmed, with a checkmark) until the parent session ends or the user collapses it. When a worker fails, it stays visible (red) until acknowledged.

**REQ-011:** Each worker child entry shows: agent type icon (Claude/Gemini/Codex), task ID or session name, target files or description, current status (queued/working/committed/failed), and elapsed time.

**REQ-012:** Clicking a worker entry opens a detail drawer (bottom panel or right-side panel, Docker Desktop style) showing that worker's event stream (all AgentStreamEvents for that session, filtered client-side from the WebSocket firehose). This is not an interactive terminal — it is a structured activity log. The drawer is separate from the sidebar to prevent the tree from becoming unreadably tall during fleet builds. The sidebar stays dense and scannable; the drawer provides depth. Hierarchy is capped at 3 visible levels (Session → Worker → sub-worker details in drawer). If a sub-worker fails, its parent worker's status bubbles red.

**REQ-013:** The sidebar has a bottom section called **Unmanaged** that lists Claude/Gemini/Codex processes discovered via local process table scan (every 5 seconds) that are NOT managed by the daemon and NOT Pantheon terminal panels. Each shows PID, project directory (from cwd), RSS memory, and idle time. These are the orphaned Terminal.app sessions.

**REQ-014:** Unmanaged sessions show a warning badge if idle for more than 30 minutes or if RSS exceeds 300MB. The user can click "Kill" which shows a confirmation dialog: "This will forcefully terminate the process. The Terminal.app window may be left in a broken state." On confirm, Pantheon sends SIGTERM to the process. If macOS blocks process introspection (sandbox or permission restrictions), the Unmanaged section degrades gracefully — it shows "Process scanning unavailable" instead of crashing or requesting elevated privileges.

### Status Area — Metrics and Fleet

**REQ-015:** The status area (right panel) has three collapsible sections: **Token Economics**, **Fleet Status**, and **System Health**.

**REQ-016:** Token Economics shows token volume as the primary metric (not dollar cost — agents are CLI/subscription). Display format per session: "248K in / 31K out / 71% cached". Aggregate display: "Today: 1.2M in / 180K out". Updated after every TurnCompleted event received via WebSocket. Data comes from the daemon's `/api/tokens/*` endpoints.

**REQ-017:** Fleet Status shows all active ABE tasks: task ID, assigned files, status (queued/working/committed/failed), and elapsed time. Data comes from the daemon's `/api/fleet/*` endpoint (NEW — does not exist yet, must be added to daemon for v4.0). Tasks move through statuses in real-time as the build progresses.

**REQ-018:** System Health shows: total agent process count, total Physical Footprint memory across all agent processes (NOT RSS — RSS double-counts shared libraries; use macOS `footprint` / `vmmap --summary` for accurate per-process cost), system memory percentage used, and a memory pressure bar. If total agent memory exceeds 60% of system RAM, the bar turns red and a notification suggests killing idle sessions. Process scanning uses `libproc` / `proc_pidinfo` with `PROC_PIDVNODEPATHINFO` for cwd discovery (same-user processes only, no root needed). Scans for agent processes by reading full command-line arguments (via `sysinfo` crate or `KERN_PROCARGS2` sysctl), not just the executable name — Claude Code may appear as `node /path/to/claude` or `bun /path/to/claude`, not just `claude`. Match against both executable basename AND script path in argv.

### Daemon Integration

**REQ-019:** Pantheon manages the daemon connection through a four-state health machine: `starting` → `ready` → `degraded` → `disconnected`. On launch, Pantheon checks `http://127.0.0.1:8080/health`. If not running, it spawns the bundled daemon binary as a **detached process** (NOT a Tauri-managed sidecar — Tauri sidecars are killed on app quit, but the daemon MUST survive Pantheon closing per REQ-023). The daemon is spawned with `Command::new().spawn()` using OS-level daemonization flags so it continues running independently. Pantheon then enters `starting` state (menubar icon: pulsing animation). When `/health` returns 200 and WebSocket connects, state transitions to `ready` (filled circle icon). If WebSocket drops but HTTP still responds, state is `degraded` (circle-with-exclamation icon — sidebar shows "WebSocket disconnected, polling for data"). If both fail, state is `disconnected` (circle-with-slash icon — reconnect with exponential backoff, max 10 seconds). All icons are template images (shape-based, not color-based) per REQ-003. Terminal panels remain functional in all states except that the sidebar Workers section goes stale during `degraded`/`disconnected`.

**REQ-020:** Pantheon maintains a persistent WebSocket connection to `ws://127.0.0.1:8080/ws`. It tracks `lastSeenSeq` from AgentStreamEvent sequence numbers. On reconnect: if gap <30 seconds, request replay from `lastSeenSeq` via the daemon. If gap >30 seconds or daemon returns "out of buffer," perform a full state refresh via `/api/state` REST endpoint (NEW — returns current sessions, workers, hierarchy, status). Client-side filtering: Pantheon receives all events and filters by session_id in memory (`Map<session_id, Vec<AgentStreamEvent>>`). Server-side filtered subscriptions deferred to v4.1.

**REQ-021:** Pantheon polls the daemon's HTTP endpoints for data not available via WebSocket: `/api/tokens/*` (every 5 seconds), `/api/fleet/*` (every 3 seconds), `/health` (every 10 seconds).

### Keyboard Shortcuts

**REQ-022:** The app uses standard macOS keyboard conventions. No vim-mode. No custom modifier keys. No tmux prefix chords. Native macOS Edit menu is customized: Find menu item is OMITTED so `Cmd+F` passes through to xterm.js addon-search instead of triggering WKWebView's native find-in-page. Use `tauri-plugin-prevent-default` as a fallback for other browser shortcuts (Print, Save). Cmd+F added to shortcut table below.

| Action | Shortcut |
|---|---|
| New terminal panel | Cmd+T |
| Close terminal panel | Cmd+W |
| Split horizontally | Cmd+D |
| Split vertically | Cmd+Shift+D |
| Next tab | Cmd+Shift+] |
| Previous tab | Cmd+Shift+[ |
| Toggle sidebar | Cmd+B |
| Toggle status area | Cmd+Shift+B |
| Focus terminal | Cmd+1 through Cmd+9 |
| Kill selected unmanaged session | Cmd+K |
| Quit Pantheon | Cmd+Q |

**REQ-023:** Cmd+Q quits Pantheon. If any terminal panel has an active Claude Code session (not idle), Pantheon shows a confirmation dialog: "X sessions still active. Quit anyway?" On confirm, sends SIGTERM to all Claude Code child processes spawned by Pantheon terminal panels. It does NOT stop the daemon or kill daemon-spawned workers.

### Performance

**REQ-024:** Memory is budgeted in two tiers. The app shell (Tauri + WKWebView + Svelte UI + sidebar + status area) must stay under 100MB RSS. Each xterm.js terminal panel is budgeted at 60MB (scrollback capped at 5,000 lines, WebGL addon disposed on background/hidden tabs and re-initialized on focus). 5 active panels = ~300MB panels + ~100MB shell = ~400MB total. These budgets are enforced via profiling gates during development, not hard runtime limits. This is separate from the Claude Code processes running inside those panels.

**REQ-025:** The app must launch to a usable state (window visible, sidebar populated) within 3 seconds on an M4 MacBook Pro.

**REQ-026:** xterm.js terminal panels must render at 60fps during active Claude Code streaming. No visible lag when Claude outputs large code blocks.

### Build and Distribution

**REQ-027:** Pantheon is built with Tauri v2. The frontend is Svelte (reusing patterns from the existing `dashboard/` codebase). The backend is Rust (sharing crates with the daemon where possible, especially `shared-types` for AgentStreamEvent).

**REQ-028:** The app is distributed as an unsigned `.dmg` for macOS. Code signing and notarization are deferred to a future version when the app is distributed to external users. For v4.0, the user right-clicks → "Open" to bypass Gatekeeper on first launch. The install target is `~/Applications/Pantheon.app` or `/Applications/Pantheon.app`. The `triumvirate` daemon binary is bundled inside the `.app` bundle (`Contents/Resources/` or `Contents/MacOS/`). Pantheon spawns it as a detached process (NOT a Tauri sidecar — see REQ-019) so it survives Pantheon quitting. An "external daemon override" setting allows power users to point Pantheon at a separately-installed daemon.

**REQ-029:** Pantheon compiles for macOS aarch64 (Apple Silicon). Intel Mac support is not a target for v4.0. Linux and Windows are not targets for v4.0. The `pantheon/src-tauri/` crate is a member of the daemon's Cargo workspace (`daemon/Cargo.toml`). `shared-types` is referenced via `path = "../../daemon/crates/shared-types"`. One `Cargo.lock` at repo root.

**REQ-030:** The `pantheon` CLI command uses `tauri-plugin-single-instance` + `tauri-plugin-deep-link` with a `pantheon://` URL scheme. Running `pantheon` from any terminal (via `open -a Pantheon` or `open pantheon://`) opens the app or focuses the existing instance. The single-instance plugin callback receives CLI args and can trigger actions in the running app. A symlink at `/usr/local/bin/pantheon` is created by the install script.

---

## User Stories

**US-1: First Launch.** Mike installs Pantheon.app. He opens it from the Dock. The app window appears with an empty terminal area. The sidebar shows "Workers: none" and "Unmanaged: scanning..." After 2 seconds, the sidebar populates: it finds 3 existing Claude Code processes in Terminal.app windows (unmanaged). The daemon auto-starts. Mike presses Cmd+T, picks ~/projects/triumvirate, and Claude Code starts in the terminal panel. He has one app with one Claude session and visibility into his 3 orphaned Terminal.app sessions.

**US-2: Fleet Build.** Mike tells Claude to run a 6-task fleet build. The Workers section populates automatically: T-001 through T-006 appear as they're dispatched. Status updates in real-time. Token Economics ticks up. One worker fails — it turns red. Mike clicks it to see the error event stream. He never left Pantheon.

**US-3: Memory Pressure.** After 4 hours, System Health shows 28GB / 36GB used. The memory bar is red. The Unmanaged section shows 4 idle Claude sessions in Terminal.app holding 1.8GB combined. Mike clicks "Kill" on each one. Memory drops to 22GB. He closes the Terminal.app windows now that the processes are dead.

**US-4: Multi-Project.** Mike has two projects going. He presses Cmd+T for a second terminal panel. Picks ~/projects/gtm-machine. Now he has two tabs — "triumvirate" and "gtm-machine". He switches between them with Cmd+1 and Cmd+2. Both projects' workers show in the same sidebar.

**US-5: Daemon Goes Down.** The daemon crashes. The menubar icon turns yellow. The sidebar shows "Daemon disconnected." Terminal panels keep working — Claude Code runs in PTYs independent of the daemon. Workers section goes stale. When the daemon restarts, connection resumes and workers repopulate.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Pantheon (Tauri v2 App)                   │
│                                                             │
│  ┌─ Sidebar ──────┐  ┌─ Terminal Area ──┐  ┌─ Status ────┐ │
│  │ My Sessions    │  │                  │  │ Tokens      │ │
│  │  triumvirate   │  │  ┌────────────┐  │  │ $0.42 sess  │ │
│  │  gtm-machine   │  │  │ xterm.js   │  │  │ $3.18 build │ │
│  │                │  │  │            │  │  │             │ │
│  │ Workers        │  │  │ Claude     │  │  │ Fleet       │ │
│  │  codex-w1 ✓    │  │  │ Code PTY   │  │  │ T-001: ✓    │ │
│  │  codex-w2 ●    │  │  │            │  │  │ T-002: ●    │ │
│  │  gemini idle   │  │  │            │  │  │ T-003: ○    │ │
│  │                │  │  └────────────┘  │  │             │ │
│  │ Unmanaged      │  │                  │  │ System      │ │
│  │  PID 40877     │  │  [tab1] [tab2]   │  │ 22GB/36GB   │ │
│  │  485MB ⚠ idle  │  │                  │  │ ▓▓▓▓▓░░░░░  │ │
│  └────────────────┘  └──────────────────┘  └─────────────┘ │
└─────────────────────────────────────────────────────────────┘
         │                      │                    │
         │ Process scan         │ PTY                │ WebSocket + HTTP
         ▼                      ▼                    ▼
   libproc/sysinfo       Claude Code CLI      Triumvirate Daemon
   (Physical Footprint)   (child process)      (ws://127.0.0.1:8080)
                          (child process)      (ws://127.0.0.1:8080)
```

**Data flows:**
- Terminal panels: PTY bidirectional with Claude Code child processes
- Workers: WebSocket subscription to daemon `/ws` (AgentStreamEvent)
- Token Economics: HTTP polling daemon `/api/tokens/*` (5s)
- Fleet Status: HTTP polling daemon `/api/fleet/*` (3s)
- System Health: Local process table scan via `ps` (5s)
- Unmanaged sessions: Process scan filtered for claude/gemini/codex PIDs (5s)

**Crate/package structure:**
- `pantheon/` — new top-level directory in the triumvirate repo
- `pantheon/src-tauri/` — Rust backend (Tauri commands, daemon client, process scanner)
- `pantheon/src/` — Svelte frontend (reusing patterns from `dashboard/`)
- `pantheon/src-tauri/` depends on `shared-types` for AgentStreamEvent deserialization
- `pantheon/src/` uses xterm.js for terminal panels, connects to Tauri backend via IPC

**Does NOT depend on:** `daemon-core`, `daemon-http`, `mcp-bridge`, or any daemon internals. Pantheon talks to the daemon over its public HTTP/WebSocket API only.

---

## Tech Stack

| Component | Technology | Version | Rationale |
|---|---|---|---|
| App framework | Tauri v2 | latest stable | Rust backend matches daemon. Smaller than Electron. Native macOS integration. |
| Frontend framework | Svelte | 5.x | Dashboard already uses Svelte. Team knowledge exists. |
| Terminal emulator | xterm.js | latest stable | Industry standard. Cursor, VS Code, Warp all use it. Claude Code works in it. |
| PTY management | `tauri-plugin-pty` or `portable-pty` (pure Rust) | latest | Spawns Claude Code child processes with pseudo-terminals. NOT node-pty (requires Node.js, defeats Tauri's purpose). |
| WebSocket client | Svelte store + native WebSocket | — | Subscribes to daemon /ws for AgentStreamEvents. |
| HTTP client | fetch | — | Polls daemon REST endpoints for tokens, fleet, health. |
| Process scanning | Rust (sysinfo crate) or `ps` subprocess | — | Discovers unmanaged agent processes, reads RSS. |
| Styling | Tailwind CSS | 4.x | Consistent with dashboard. Fast iteration. |
| Build | Tauri CLI + Vite | — | Standard Tauri v2 build pipeline. |
| Distribution | .dmg (unsigned for v4.0) | — | macOS standard. Code signing deferred per REQ-028. |

---

## What the Daemon Already Provides (v3.3.0)

- `/ws` WebSocket — `agent_stream` events with AgentStreamEvent (6 variants, seq numbers, display_text())
- `/health` — daemon state
- `/api/tokens/*` — token economics (per-session, per-build cost tracking)
- ABE task tracker — fleet dispatch, worker status, task lifecycle
- Agent sessions — spawn, ask, dismiss, list
- MCP proxy — Claude API proxy for inter-agent communication

## Release Strategy

**v3.9.0 — Daemon Backend for Pantheon** (build first)
- All daemon API additions listed below
- `parent_session_id` + `root_session_id` lineage tracking
- Event replay ring buffer
- New REST endpoints (`/api/workers`, `/api/fleet/*`, `/api/state`)
- `WorkerLifecycle` WebSocket events
- PID file management
- `PANTHEON_SESSION_ID` capture via MCP `_meta` and HTTP headers
- Ship, test, verify the API surface independently
- Existing `triumvirate watch` CLI can exercise the new endpoints

**v4.0.0 — Pantheon Tauri App** (build against v3.9.0 APIs)
- Tauri v2 + Svelte 5 + xterm.js
- Consumes v3.9.0's API surface — no daemon changes needed
- All REQs in this spec that describe the GUI

---

**What the daemon needs to ADD for Pantheon (v3.9.0 scope):**
- `/api/workers` endpoint — list all active daemon-managed sessions/workers with their current status, `parent_session_id`, `root_session_id`
- `/api/fleet/*` endpoint — ABE task status (queued/working/committed/failed) per task
- `/api/state` endpoint — full current state snapshot for reconnect (sessions, workers, hierarchy, fleet status)
- `parent_session_id` + `root_session_id` fields on every worker record, captured from MCP caller session context during dispatch, persisted in SQLite
- `WorkerLifecycle` variant added to `AgentStreamEvent` with `spawned`/`completed`/`failed` sub-types carrying lineage fields
- Event replay ring buffer: `VecDeque<AgentStreamEvent>` capped at 1,000 events. On WebSocket connect, client sends `lastSeenSeq`. Daemon replays events with `seq > lastSeenSeq` from buffer, then switches to live broadcast. If `lastSeenSeq` is older than the buffer's oldest event, daemon responds with `{"replay": "out_of_range"}` and client falls back to full `/api/state` REST refresh. Buffer memory cost: ~200KB (1000 events × ~200 bytes). Subscribe is acquired BEFORE reading buffer to prevent gap between history and live stream.
- Version field in `/health` response (already exists) — Pantheon reads it for version handshake
- Daemon token at `~/.triumvirate/daemon.token` (already exists via `ensure_daemon_token()`) — Pantheon reads this file for bearer auth
- PID file at `~/.triumvirate/daemon.pid` with `flock` locking (via `pidfile-rs`). Atomic write/rename. Pantheon verifies PID matches `triumvirate` binary via `libproc` before sending signals (PID recycling protection). Stale PID recovery: if file exists but lock is released, daemon crashed — safe to overwrite.

---

## Additional Requirements (from Rounds 2-7)

### Dark Mode

**REQ-031:** Pantheon auto-detects system dark/light mode via Tauri's `getCurrentWindow().onThemeChanged()` event. Svelte 5 `$effect` toggles Tailwind `dark:` classes. xterm.js terminal theme object is updated on theme change. White flash prevention: window starts with `visible: false` in `tauri.conf.json`, shown after theme logic runs in `onMount`. Three modes: Light, Dark, Auto (sync with system). Preference stored via `tauri-plugin-store`.

### Persistence

**REQ-032:** User preferences (theme mode, daemon URL override, Claude binary path) stored via `tauri-plugin-store` at `~/Library/Application Support/com.pantheon.app/settings.json`. Window size/position auto-saved/restored via `tauri-plugin-window-state`. Panel split layout stored in the same settings.json. No `localStorage` for critical settings — flat JSON files for debuggability.

### Session Linking

**REQ-033:** Pantheon sets a `PANTHEON_SESSION_ID` environment variable on each Claude Code child process (unique per terminal panel). This env var propagates to all child processes including the MCP proxy. The linking mechanism works across BOTH MCP transports:
- **HTTP/SSE transport (proxy mode):** The proxy reads `PANTHEON_SESSION_ID` from its environment and sends it as an `X-Pantheon-Session-Id` HTTP header on every request to the daemon.
- **stdio transport (mcp mode):** The `triumvirate mcp` command reads `PANTHEON_SESSION_ID` from its environment and injects it into the MCP `initialize` request's `_meta` field: `_meta: { "pantheon.session_id": "<value>" }`. The daemon's MCP handler reads this during initialization.
The daemon's `open_session(panel_id, pid, cwd)` handshake returns a canonical `session_id` that durably links the Pantheon panel to the daemon session. Workers dispatched from that session inherit `parent_session_id`. This linkage survives daemon restarts because both IDs are persisted in SQLite. If `PANTHEON_SESSION_ID` is absent (non-Pantheon sessions), the daemon assigns workers to an "unlinked" root — they appear in the sidebar under "Daemon Workers" instead of under a specific terminal panel.

### Background Behavior

**REQ-035:** When the main window is hidden (closed to menubar tray per REQ-003), all PTY processes continue running. Claude Code sessions remain active in the background. If Claude Code appears to be waiting for user input while the window is hidden, Pantheon sends a macOS native notification (via `tauri-plugin-notification`): "Claude needs your input in [project name]." Detection is best-effort for v4.0: PTY idle >5 seconds AND last output line (ANSI-stripped) matches prompt patterns (`? `, `[Y/n]`, `(y/N)`). Debounced to one notification per session per minute. Per-agent regex profiles configurable. User can snooze/disable notifications in preferences. False positives during long Claude thinking pauses are acceptable — the notification is a hint, not a guarantee. Shell integration via OSC sequences is the v5.0 path for reliable detection. Clicking the notification reopens and focuses the main window.

### Layout

**REQ-034:** The three-region layout (sidebar, terminal area, status area) uses CSS flexbox. The sidebar and status area are collapsible fixed-width panels. The terminal area fills the remaining space. Within the terminal area, PaneForge (Svelte 5 component) manages tab and split layouts with built-in persistence. The outer flexbox layout is NOT managed by PaneForge.
