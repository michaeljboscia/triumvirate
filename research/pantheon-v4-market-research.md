# Pantheon v4.0 — Market & Technical Research

**Date:** 2026-04-11  
**Source:** 9 Gemini searches (live web data)  
**Purpose:** Inform goatrodeo architecture rounds for the Pantheon TUI spec  

---

## 1. Ratatui Ecosystem (2025-2026 Best Practices)

### Component-Action Architecture (dominant pattern)
- Components emit `Action` enums via `mpsc` channels, central `update` loop processes them
- Each component implements: `handle_key_events()`, `update()`, `draw()`
- Mirrors GitUI modularity — unit-test a single panel without booting the app

### Focus Management
- **Focus Stack** (not simple index) — panels pushed/popped like a stack
- `ratatui-interact` or `rat-focus` crates handle Tab/Shift+Tab cycling
- Active panel: `BorderType::Thick` or distinct color (Yellow/Cyan)
- Inactive panels: dimmed

### Layout
- Nested constraint solvers: `Constraint::Fill(1)`, `Constraint::Min(x)`
- Responsive to terminal resize without flickering
- **Dirty flag rendering** — only redraw when state changed or on Tick (250ms for monitors)

### Keyboard Navigation
- **Modal navigation** (Vim/LazyGit style): Normal, Insert, Filter, Command modes
- Normal: `h/j/k/l` or arrows; `q` to go back
- Command mode: `:` opens footer input
- Focus cycling: Tab/Shift+Tab with `rat-focus`

### Async Architecture
- Main thread handles terminal + rendering
- Tokio runtime in background for I/O
- State sync via channels (send ViewModels to UI), not `Arc<Mutex<AppState>>`
- "Ghost States" — loading spinners during async operations

### Recommended 2025 Stack
- Framework: `ratatui-templates/component` base
- Input: `crossterm` backend + `ratatui-textarea` for text input
- Focus: `ratatui-interact` for mouse + tab
- Template: Component + Action pattern

---

## 2. AI Agent Orchestration UIs (2025-2026 Trends)

### Visual Language: "Command & Control"
- Deep charcoal backgrounds + high-contrast status colors
- Status colors: #00FFA3 (executing), #FFB800 (awaiting), #FF3D00 (halt)
- Micro-interactions: pulse on active agents, glitch for alerts

### Multi-Agent Components
- **Agent Identity Cards** — miniature panels per agent: state, token usage, burn rate, persona
- **Unified Swarm Log** — vertical event stream, color-coded by agent ID
- **Tool-Call Breakouts** — sub-terminal pops up for tool I/O

### Live Streaming Patterns
- **Thought Stream** — dimmed secondary column for reasoning, bold for output
- **Token Visualizer** — scrolling waterfall or bar graph for TPS
- **Streaming Action Blocks** — agent's internal monologue streams, user can intervene

### Intervention Controls
- Kill Switch / Pause ribbon — persistent global controls
- HITL gates — amber text + review button for high-risk steps
- Dynamic parameter tuning — sliders for temperature/autonomy

---

## 3. Terminal Multiplexers (Zellij vs tmux)

### Zellij (Modal, Discoverable)
- Modes: Pane, Tab, Resize — status bar updates per mode
- No prefix key — direct Alt+shortcuts
- Floating panes (native, persistent)
- Stacked panes (tabs within a split)
- Layout isolation (resize doesn't break siblings)
- Full mouse-drag support
- v0.42+ "Swap Layout" — instant layout switching without closing panes

### tmux (Prefix, Minimal)
- Prefix key (`Ctrl-b`) + chord
- No persistent UI help
- v3.5+ improved popups/menus
- Mirrored layouts for wide-screen
- Lower resource footprint (good for remote/SSH)

### Design Lessons for Pantheon
- Zellij's mode-aware status bar = discoverability win
- Focus isolation: only active pane receives input
- Layout save/restore is table stakes
- Tab cycling needs visual feedback (Zellij's approach is best)

---

## 4. IDE Terminal Layouts (VS Code, Cursor, Zed, Warp)

### "Agent Mission Control" Pattern (emerging 2026)
- Terminal as dynamic orchestrator, not side-panel
- VS Code v1.109+: parallel Agent Sessions view (grouped by local/background/cloud)
- Inline terminal streaming: command output renders inside chat sidebar
- Detachable terminals for multi-monitor

### Multi-Terminal Management
- **Terminal-as-Code** — `.vscode/tasks.json` auto-launches color-coded terminals
- **Grid Pattern** — 2x2/4x4 terminal matrices in editor area
- **Sticky Scroll** — command context pinned during long output
- **Block Pattern (Warp)** — output partitioned into command/output blocks, individually shareable

### Warp's Three-Pane Split
- Left: long-running process logs
- Right: steerable agent workspace
- Bottom: production logs / SSH

### Design Lessons for Pantheon
- Warp's "Block" concept (command+output pairs) is excellent for event grouping
- VS Code's "Agent Sessions" grouping maps to our Fleet Status
- Inline streaming (results flow into chat, not separate panel) — interesting alternative
- Ghost text / streaming text is expected UX for AI tools now

---

## 5. Claude Code Terminal Architecture (Critical for PTY Wrapping)

### Rendering Stack
- **React + Ink** — reactive component model for terminal
- **Yoga layout engine** — Flexbox in TTY (margins, padding, flex-direction)
- **Custom ANSI parser stack** — full CSI/DEC/ESC/OSC support
- Claude Code IS a TUI application, not a simple CLI

### PTY Architecture
- Spawns commands in PTY slave (tricks child into thinking it's interactive)
- **Buffer interception** — man-in-the-middle for PTY stream
- Input relay: manual forwarding of keystrokes from real terminal to PTY
- Uses Bun's event loop for concurrent PTY stream management

### The Wrapping Problem (CRITICAL for Pantheon)
- **Virtual height calculation** — must subtract headers/footers from viewport
- **Hard vs soft wrapping** — Claude Code hard-wraps at column width
- **Reflow on resize** — React re-render + Yoga recalculation + PTY history reflow
- **Alternate Screen Buffer** — child TUIs (vim, top) try to clear parent screen

### Sub-Agent Handling
- "Terminal inside a terminal" — encapsulated TUIs
- Alternate Screen Buffer suppression (prevent child from clearing parent)
- Status line integration via COLUMNS/LINES env vars
- "Smart Scrolling" — active output doesn't jump scroll position

### IMPLICATIONS FOR PANTHEON
Claude Code is a complex TUI (React+Ink+Yoga), NOT a simple CLI.
Wrapping it in another TUI (Ratatui) creates a "TUI-in-TUI" problem:
- Claude Code expects to own the terminal (alternate screen, raw mode)
- Escape sequences from Claude Code may conflict with Ratatui's rendering
- Resize coordination between Ratatui, vt100 parser, AND Claude Code's internal Yoga layout
- Claude Code's `--bare` mode may be necessary to strip its TUI wrapper

---

## 6. Warp Terminal Design (ADE Pattern)

### Agent Management Panel
- Centralized monitoring of all active AI agents
- Three-pane split: logs | agent workspace | production monitoring
- Tabbed agent contexts with "Shared Substrate" for cross-agent data

### Ghost Text & Streaming
- Intent-aware ghost text (predicts multi-command pipelines)
- Streaming Action Blocks — agent monologue streams, user can steer
- Frosted glass / Liquid Glass overlays for depth hierarchy

### Intervention UX
- Plan Block (`/plan`) — editable roadmap before execution
- Side-by-side visual diffs within terminal blocks
- Universal Input field — auto-detects natural language vs commands

### Design Lessons for Pantheon
- Warp's "Plan Block" pattern = our Fleet Status showing task plan
- Streaming blocks with intervention capability = Live Agents with steer option
- Universal Input = our Conversation panel (natural language to Claude)

---

## 7. PTY Embedding in Ratatui (Technical Deep Dive)

### The Stack
```
portable-pty → spawns child process in PTY
     ↓
vt100::Parser → maintains terminal state (buffer, colors, cursor)
     ↓
tui-term::PseudoTerminal → renders vt100::Screen into Ratatui buffer
```

### Key Crates
- `tui-term` v0.3 — the standard widget, includes `vt100` and `portable-pty` as optional features
- `portable-pty` v0.9 — from WezTerm project, cross-platform
- `vt100` — maintains screen state, cursor position, colors

### Dependencies (Cargo.toml)
```toml
ratatui = "0.30"
crossterm = "0.29"
tokio = { version = "1", features = ["full"] }
tui-term = { version = "0.3", features = ["portable-pty", "vt100"] }
portable-pty = "0.9"
```

### Critical Implementation Details
1. **Async is mandatory** — PTY output is unpredictable, must use tokio::select!
2. **Resize coordination** — must update BOTH vt100 parser AND PTY master on resize
3. **Input forwarding** — tui-term only renders; input must be manually written to PTY master
4. **Arc<Mutex<Parser>>** — shared between async reader thread and UI render loop
5. **Dirty flag** — only redraw when parser receives new bytes, not every frame
6. **Alternative: `ansi-to-tui`** — simpler but no cursor movement or interactive apps

### The Claude Code Wrapping Problem (RISK)
Claude Code uses React+Ink+Yoga for its TUI. Wrapping it:
- Option A: Full PTY wrap with vt100 — must handle Claude Code's alternate screen, escape sequences, and internal layout engine. Complex but transparent.
- Option B: `claude --bare` mode — strips Claude Code's TUI, outputs raw text. Simpler but loses Claude's formatting (markdown, code blocks, status line).
- Option C: Don't embed — run Claude Code in a SEPARATE pane/terminal and Pantheon shows only monitoring panels. Sidesteps the wrapping problem entirely.

---

## 8. k9s Architecture (Real-Time Dashboard Reference)

### Architecture
- Go + tview (tcell backend)
- Decoupled DAO (K8s API) from View Layer
- Kubernetes Informers (Watch API) — push-based, not polling
- Delta FIFO Queue for local cache updates
- Dirty flag + scheduled refresh for rendering

### Refresh Strategy
- Resource data: **push-based** via Watch API (sub-second perceived latency)
- Metrics (CPU/RAM): **poll-based** at configurable interval
- Default UI refresh: **2 seconds**
- High-frequency event **debouncing** for large clusters (1000+ pods)

### Design Lessons for Pantheon
- Push-based (WebSocket) for events, poll-based for aggregate data (costs, fleet status)
- Dirty flag rendering matches ratatui best practice
- 2-second refresh for "stats" panels is industry standard
- Debouncing for event storms is mandatory

---

## Synthesis: Critical Design Decisions for Spec

### 1. PTY Wrapping is the Riskiest REQ
Claude Code is a TUI, not a CLI. Wrapping it creates TUI-in-TUI complexity.
Three options with different risk/reward profiles need architecture-round analysis.

### 2. Component-Action Pattern is Non-Negotiable
Every modern ratatui app uses it. The spec should mandate this architecture.

### 3. Focus Stack, Not Tab Index
The spec says Tab cycles focus. Research says use a Focus Stack (push/pop).
Tab cycling is fine as the default, but the implementation should be a stack.

### 4. Dirty Flag Rendering
The spec says 30fps. Research says "render on change, not on tick."
Both can coexist: tick at 30fps but only redraw dirty panels.

### 5. Push Events + Poll Stats
WebSocket for agent events (push, sub-100ms). HTTP poll for costs and fleet (2-5 second interval).
This matches k9s pattern and is already in the spec.

### 6. Debouncing for Fleet Builds
A 24-task fleet build generates hundreds of events. Need event debouncing in Live Agents panel.

### 7. Zellij-Style Mode Awareness
Status bar should show available keybindings based on current focus/mode.
Massively reduces learning curve.
