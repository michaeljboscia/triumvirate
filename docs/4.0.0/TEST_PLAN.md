# Pantheon v4.0 — Test Plan

**Spec:** specs/PANTHEON_V4.md  
**PRD:** docs/4.0.0/PRD.md  

---

## Test Matrix

Every REQ has at least one test. Every test has a pass condition that a stub cannot satisfy.

| REQ-ID | FEAT-ID | Acceptance Criteria | Test Type | Pass Condition | Pre-Implementation Baseline |
|--------|---------|-------------------|-----------|---------------|---------------------------|
| REQ-001 | FEAT-007 | App launches from Dock/Spotlight/CLI within 3s | E2E | Window visible + sidebar populated < 3s measured via Instruments | No app exists |
| REQ-002 | FEAT-007 | Three-region layout with collapsible sidebar/status | E2E | Sidebar toggles with Cmd+B, status with Cmd+Shift+B, flexbox layout verified | No app exists |
| REQ-003 | FEAT-007 | Menubar template icons, 4 states, close-to-tray | E2E | All 4 icon states render, Cmd+W hides window, tray click reopens | No app exists |
| REQ-004 | FEAT-001 | Claude Code renders in xterm.js with full formatting | E2E | Colors, markdown, status line, tool approval UI all render identically to Terminal.app | No terminal panel exists |
| REQ-005 | FEAT-001 | Cmd+T creates terminal with directory picker + recents | E2E | Directory picker opens, recent projects dropdown shows, Claude Code starts in selected dir | No terminal panel exists |
| REQ-006 | FEAT-001 | Tabs + splits via PaneForge | E2E | Cmd+D splits horizontally, Cmd+Shift+D vertically, tabs switch with Cmd+1-9 | No terminal panel exists |
| REQ-007 | FEAT-001 | PTY with scrollback, search, WebGL, resize | Integration | 5000-line scrollback verified, Cmd+F search finds text, WebGL addon active, resize reflows | No terminal panel exists |
| REQ-008 | FEAT-001 | Session ended UI with Restart/Close | E2E | Kill Claude Code → "Session ended" + buttons appear, Restart works | No terminal panel exists |
| REQ-009 | FEAT-002 | Hierarchical sidebar tree | E2E | Workers appear indented under parent session, project name at top level | No sidebar exists |
| REQ-010 | FEAT-002 | Workers auto-populate from WebSocket with lineage | Integration | Dispatch worker → appears in sidebar <1s with correct parent_session_id | Daemon has no lineage |
| REQ-011 | FEAT-002 | Worker entry shows agent, task, status, time | E2E | All fields visible for active worker, status changes in real-time | No sidebar exists |
| REQ-012 | FEAT-002 | Worker detail drawer with event stream | E2E | Click worker → drawer opens with filtered AgentStreamEvents | No drawer exists |
| REQ-013 | FEAT-003 | Unmanaged session scanner | Integration | Start Claude in Terminal.app → appears in Unmanaged section within 5s | No scanner exists |
| REQ-014 | FEAT-003 | Kill unmanaged with confirmation | E2E | Click Kill → dialog → confirm → process dies → entry removed | No scanner exists |
| REQ-015 | FEAT-004 | Three collapsible status sections | E2E | Token Economics, Fleet Status, System Health all visible and collapsible | No status area exists |
| REQ-016 | FEAT-004 | Token volume display format | Integration | "248K in / 31K out / 71% cached" format verified against daemon data | Daemon has token endpoints |
| REQ-017 | FEAT-004 | Fleet status with real-time task updates | Integration | ABE tasks show queued→working→committed transitions in real-time | Daemon has ABE but no /api/fleet |
| REQ-018 | FEAT-003 | Physical Footprint memory + pressure bar | Integration | Memory values match Activity Monitor (±5%), bar turns red >60% | No health panel exists |
| REQ-019 | FEAT-005 | Four-state daemon health machine | E2E | Start with no daemon → starting (pulsing) → ready (filled). Kill daemon → disconnected (slash). Restart → ready | No health machine exists |
| REQ-020 | FEAT-005 | WebSocket reconnect with replay | Integration | Disconnect WS for 10s → reconnect → events replayed from lastSeq → no gap | Daemon has no replay |
| REQ-021 | FEAT-005 | REST polling intervals | Integration | /api/tokens polled every 5s, /api/fleet every 3s, /health every 10s (verified via network monitor) | No polling exists |
| REQ-022 | FEAT-007 | Standard macOS keyboard shortcuts | E2E | All shortcuts in spec table work. No vim-mode. No prefix chords. Cmd+F goes to xterm search, not WKWebView | No app exists |
| REQ-023 | FEAT-007 | Quit confirmation with SIGTERM cleanup | E2E | Cmd+Q with active session → dialog. Quit → Claude gets SIGTERM. Daemon stays running | No app exists |
| REQ-024 | FEAT-016 | Memory budget: shell <100MB, panel ~60MB | Manual | Measured via Instruments/Activity Monitor. Physical Footprint. WebGL disposed on hidden tabs | No app exists |
| REQ-025 | FEAT-016 | Launch <3 seconds | Manual | Timed from app icon click to sidebar populated. M4 MacBook Pro | No app exists |
| REQ-026 | FEAT-016 | 60fps terminal rendering | Manual | Instruments frame time <16.7ms during Claude streaming | No app exists |
| REQ-027 | FEAT-010 | Tauri v2 + Svelte 5 + shared-types | Unit | cargo check for workspace, npm run build succeeds, shared-types accessible | No Tauri app exists |
| REQ-028 | FEAT-010 | Unsigned .dmg with bundled daemon | E2E | .dmg builds, installs, launches. Daemon binary present in .app bundle. Right-click Open bypasses Gatekeeper | No app exists |
| REQ-029 | FEAT-010 | Workspace: src-tauri in daemon workspace | Unit | cargo check from repo root succeeds, Cargo.lock at root, shared-types path dep works | No Tauri app exists |
| REQ-030 | FEAT-010 | CLI command + deep-link | E2E | `open pantheon://` focuses app. `pantheon` CLI opens/focuses. Single instance enforced | No app exists |
| REQ-031 | FEAT-008 | Dark mode auto-detection + 3 modes | E2E | System dark → app dark. Toggle Light/Dark/Auto. xterm.js theme updates. No white flash | No app exists |
| REQ-032 | FEAT-008 | Preferences + layout persistence | E2E | Settings survive restart. Window position restored. Recent projects persisted | No app exists |
| REQ-033 | FEAT-006 | Session linking via env var + MCP | Integration | PANTHEON_SESSION_ID set on PTY child. Daemon captures via _meta (stdio) and header (HTTP). Workers inherit parent_session_id | No linking exists |
| REQ-034 | FEAT-007 | CSS flexbox outer layout + PaneForge inner | E2E | Three regions: sidebar (collapsible), terminal (flex:1), status (collapsible). PaneForge handles splits within terminal area only | No app exists |
| REQ-035 | FEAT-009 | Background PTY + notifications | E2E | Close window → PTY continues. Claude waits for input → notification within 10s. Click notification → window opens | No app exists |

---

## Test Execution Strategy

### Automated Tests (CI-compatible)

| Layer | Tool | Command | Covers |
|---|---|---|---|
| Rust unit | cargo test | `cargo test -p shared-types -p daemon-core -p pantheon-tauri` | Types, replay buffer, process scanner, daemon client |
| Svelte component | vitest | `cd pantheon && npm test` | Sidebar, StatusArea, TerminalPanel (mock xterm.js) |
| Frontend E2E | Playwright (against Vite dev server) | `cd pantheon && npx playwright test` | Layout, theme, keyboard shortcuts, collapsible panels |

### Manual Tests (require running daemon + real Claude Code)

| Test | What to verify | Why manual |
|---|---|---|
| Claude Code rendering fidelity | Compare xterm.js output to Terminal.app side-by-side | Visual comparison |
| 60fps rendering | Instruments frame profiling during streaming | Requires hardware profiling tools |
| Memory budgets | Activity Monitor Physical Footprint | Requires real process measurement |
| Launch time | Stopwatch from click to usable | Requires cold-start measurement |
| Background notifications | Close window, trigger Claude input prompt, check macOS notification | Requires real Claude interaction |
| Kill unmanaged | Open Terminal.app Claude, kill from Pantheon | Requires real process |

### Native E2E (require built .app)

| Test | Tool | What to verify |
|---|---|---|
| Full app lifecycle | tauri-plugin-pilot | Launch, create terminal, dispatch worker, view sidebar, quit |
| Menubar behavior | tauri-plugin-pilot | Close to tray, reopen, icon states |
| .dmg install | Manual | Mount, drag, launch, Gatekeeper bypass |
