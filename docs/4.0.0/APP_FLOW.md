# Pantheon v4.0 — Application Flow

**Spec:** specs/PANTHEON_V4.md  
**PRD:** docs/4.0.0/PRD.md  

---

## Launch Sequence

### Cold Start (first ever launch)
```
User double-clicks Pantheon.app
  → Tauri app initializes (Rust backend + WKWebView)
  → Window starts hidden (visible: false in tauri.conf.json)
  → Svelte onMount runs:
      1. Read theme from tauri-plugin-store → apply dark: class
      2. Show window (getCurrentWindow().show())
  → Rust backend:
      1. Read ~/.triumvirate/daemon.token
      2. Check http://127.0.0.1:8080/health
      3. If not running → spawn daemon as detached process
         → Enter "starting" state (pulsing menubar icon)
      4. Poll /health every 500ms until 200
      5. Connect WebSocket to ws://127.0.0.1:8080/ws
      6. Enter "ready" state (filled circle menubar icon)
  → Frontend:
      1. Sidebar renders: "My Sessions: (empty)" 
      2. Process scanner runs → populates Unmanaged section
      3. Terminal area shows: "Press Cmd+T to start a session"
      4. Status area shows: "Token Economics: —" / "Fleet: —" / "System: scanning..."
  → App fully loaded (< 3 seconds on M4 MacBook)
```

### Warm Start (daemon already running)
```
User opens Pantheon.app (or clicks menubar icon, or runs `pantheon` CLI)
  → Single instance check (tauri-plugin-single-instance)
  → If already running → focus existing window, exit new instance
  → If not running → same as cold start but skip daemon spawn
  → daemon.token already on disk, /health returns 200 immediately
  → WebSocket connects → sidebar populates with existing workers
  → tauri-plugin-window-state restores previous window size/position
  → Recent terminal panels NOT auto-restored (user creates them fresh)
```

---

## Screen: Main Window

### Layout (CSS Flexbox)
```
┌──────────┬──────────────────────────┬─────────────┐
│ Sidebar  │     Terminal Area        │ Status Area  │
│ 250px    │     flex: 1              │ 280px        │
│ min 200  │                          │ min 220      │
│          │                          │              │
│ [toggle  │  [tab bar]               │ [Token Econ] │
│  Cmd+B]  │  ┌──────────────────┐   │ [Fleet]      │
│          │  │ xterm.js panel   │   │ [System]     │
│          │  │                  │   │              │
│          │  │ Claude Code PTY  │   │ [toggle      │
│          │  │                  │   │  Cmd+Shift+B]│
│          │  └──────────────────┘   │              │
└──────────┴──────────────────────────┴─────────────┘
```

### Responsive Behavior
- Window ≥ 1200px: all three regions visible
- Window < 1200px: status area auto-collapses (user can toggle back with Cmd+Shift+B)
- Window = 900px (minimum): sidebar stays visible (250px sidebar + 650px terminal)
- User can manually toggle sidebar (Cmd+B) at any width

---

## Flow: Create Terminal Panel (Cmd+T)

```
User presses Cmd+T
  → If recent projects exist in settings.json:
      Show dropdown: recent projects (max 10) + "Browse..."
  → If no recent projects:
      Show native directory picker (tauri-plugin-dialog)
  → User selects directory
  → Rust backend:
      1. Generate unique PANTHEON_SESSION_ID (UUID v4)
      2. Spawn Claude Code PTY:
         - Command: claude (from $PATH, or custom binary from preferences)
         - cwd: selected directory
         - env: PANTHEON_SESSION_ID=<uuid>
         - PTY: via portable-pty or tauri-plugin-pty
         - Drop slave fd immediately after spawn
      3. Start PTY reader thread → emit bytes to frontend via Tauri events
  → Frontend:
      1. Create new xterm.js Terminal instance
         - scrollback: 5000
         - WebGL addon loaded (with contextlost handler)
         - FitAddon loaded
         - SearchAddon loaded
         - ResizeObserver on container
      2. Wire PTY events → term.write()
      3. Wire term.onData() → PTY stdin
      4. Add tab to tab bar: "[directory name]"
      5. Add entry to sidebar: "My Sessions > [directory name]"
      6. Update recent projects in settings.json
  → Claude Code greeting appears in terminal panel
  → User starts typing
```

---

## Flow: Split Terminal (Cmd+D / Cmd+Shift+D)

```
User presses Cmd+D (horizontal) or Cmd+Shift+D (vertical)
  → PaneForge splits the active terminal panel's container
  → New split pane:
      1. Same project directory as active panel
      2. New PTY spawned (new Claude Code instance, new PANTHEON_SESSION_ID)
      3. New xterm.js instance
      4. New sidebar entry under same project
  → PaneForge persists layout in tauri-plugin-store
```

---

## Flow: Worker Appears in Sidebar

```
Claude Code (in terminal panel) dispatches work via MCP:
  e.g., dispatch_codex_worktree, spawn_session

  → MCP proxy sends X-Pantheon-Session-Id header to daemon
  → Daemon:
      1. Links MCP session to PANTHEON_SESSION_ID
      2. Dispatches worker (Codex/Gemini/Claude sub-agent)
      3. Records parent_session_id = PANTHEON_SESSION_ID
      4. Emits WorkerLifecycle::Spawned via WebSocket
  → Pantheon WebSocket client receives event
  → Frontend:
      1. Finds parent session in sidebar tree by PANTHEON_SESSION_ID
      2. Adds worker as indented child:
         "codex  T-001  src/auth.rs  queued"
      3. Worker status updates as more events arrive:
         queued → working → committed ✓ (or failed ✗)
      4. TurnCompleted events update Token Economics
  → Worker completes:
      1. WorkerLifecycle::Completed received
      2. Sidebar entry dims, shows checkmark
      3. Entry stays visible until parent session ends or user collapses
  → Worker fails:
      1. WorkerLifecycle::Failed received
      2. Sidebar entry turns red
      3. Parent session node also shows red indicator (status bubbling)
      4. Entry stays visible until user acknowledges
```

---

## Flow: View Worker Details

```
User clicks a worker entry in sidebar
  → Detail drawer opens (bottom panel or right panel)
  → Drawer shows filtered AgentStreamEvents for that worker's session:
      → Gemini: turn started [auth-review]
      → Gemini: calling read_file (src/auth.rs)
      → Gemini: responded (12,847 in / 1,203 out / 4.1s)
  → Events auto-scroll as new ones arrive
  → User clicks another worker → drawer content switches
  → User clicks the drawer close button → drawer closes
  → Sidebar remains dense and scannable throughout
```

---

## Flow: Kill Unmanaged Session

```
User sees unmanaged session in sidebar:
  "PID 40877  claude  ~/projects/tellus  485MB  idle 3h ⚠"

  → User clicks "Kill" button
  → Confirmation dialog:
      "This will forcefully terminate the process.
       The Terminal.app window may be left in a broken state."
      [Cancel] [Kill]
  → User clicks Kill
  → Rust backend: kill(pid, SIGTERM)
      - First: verify PID matches claude/gemini/codex via libproc
      - If verification fails: show error "Process no longer matches"
  → Next process scan (≤5 seconds): entry removed from Unmanaged list
  → System Health memory numbers update
```

---

## Flow: Daemon Disconnect

```
Daemon crashes or network issue:
  → WebSocket connection drops (onclose/onerror event)
  → Frontend:
      1. Daemon state → "degraded" or "disconnected"
      2. Menubar icon → circle-with-exclamation or circle-with-slash
      3. Sidebar Workers section header: "⚠ disconnected — reconnecting..."
      4. Fleet Status header: "⚠ stale (last update: HH:MM:SS)"
      5. Token Economics: frozen at last known values
  → Terminal panels: UNAFFECTED (PTYs are local, independent of daemon)
  → Reconnect loop: exponential backoff (500ms, 1s, 2s, 4s, 8s, 10s max)
  → On reconnect:
      1. Check gap duration
      2. If < 30 seconds: send lastSeenSeq, daemon replays from ring buffer
      3. If ≥ 30 seconds or "out_of_range": GET /api/state for full refresh
      4. Sidebar repopulates, state → "ready"
      5. Menubar icon → filled circle
```

---

## Flow: Close Window (Cmd+W)

```
User presses Cmd+W (or clicks red X)
  → on_window_event(CloseRequested):
      1. api.prevent_close()
      2. window.hide()
  → App stays running in menubar
  → All PTY processes continue
  → WebSocket connection stays alive
  → Process scanner continues
  → If Claude Code prompts for input while window hidden:
      1. PTY idle >5s AND last line matches prompt pattern
      2. macOS notification: "Claude needs your input in [project]"
      3. Click notification → window.show() + set_focus()
```

---

## Flow: Quit (Cmd+Q)

```
User presses Cmd+Q
  → If any terminal panel has active Claude Code session:
      Confirmation dialog: "X sessions still active. Quit anyway?"
      [Cancel] [Quit]
  → On quit:
      1. Send SIGTERM to all Claude Code child processes (PTY children)
      2. Disconnect WebSocket
      3. Do NOT stop the daemon (it's a detached process)
      4. Do NOT kill daemon-spawned workers
      5. tauri-plugin-window-state saves window position/size
      6. App exits
  → Daemon continues running independently
  → Workers continue if in progress
```

---

## Flow: Theme Change

```
System dark/light mode changes (macOS System Settings)
  → Tauri fires onThemeChanged event
  → Svelte $effect:
      1. If mode === 'auto': update effectiveTheme
      2. Toggle document.documentElement.classList 'dark'
      3. Update each xterm.js instance theme object:
         - dark: { background: '#1a1b26', foreground: '#c0caf5' }
         - light: { background: '#ffffff', foreground: '#1a1b26' }
  → Tailwind dark: classes auto-apply to all components
  → No page reload needed
```

---

## Error States

| Condition | User Sees | Recovery |
|---|---|---|
| Daemon won't start | Menubar: slash icon. Sidebar: "Daemon failed: [error]" | Terminal panels still work. User can fix daemon manually. |
| Claude binary not found | Terminal panel: "Error: claude not found in PATH" + Retry button | User installs Claude Code or sets custom binary path in preferences. |
| WebGL context lost | Terminal briefly blank, then re-renders | Automatic: webglcontextlost handler disposes and re-creates addon. |
| Process scan blocked | Unmanaged section: "Process scanning unavailable" | No crash. Feature degrades silently. |
| Daemon token missing | "Cannot authenticate with daemon" in status area | User runs daemon manually or checks ~/.triumvirate/ |
| PTY crash (Claude segfault) | Terminal panel: "Session ended" + Restart/Close buttons | PTY EOF detected via read() returning 0. Automatic. |
