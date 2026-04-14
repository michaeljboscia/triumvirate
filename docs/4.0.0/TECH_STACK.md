# Pantheon v4.0 — Tech Stack

**Spec:** specs/PANTHEON_V4.md  
**PRD:** docs/4.0.0/PRD.md  

---

## Release Tracks

| Version | Scope | Stack |
|---|---|---|
| v3.9.0 | Daemon backend upgrades | Rust (existing daemon crates) |
| v4.0.0 | Tauri native Mac app | Tauri v2 + Svelte 5 + xterm.js |

---

## v4.0.0 — Pantheon App

### App Framework

| Component | Package | Version | Purpose |
|---|---|---|---|
| Desktop framework | `tauri` | 2.x (latest stable) | Native macOS app with WKWebView |
| Build CLI | `@tauri-apps/cli` | 2.x | Dev server, build, bundle |
| Vite | `vite` | 6.x | Frontend bundler |

### Frontend

| Component | Package | Version | Purpose |
|---|---|---|---|
| UI framework | `svelte` | 5.x | Reactive components with runes ($state, $effect, $props) |
| Styling | `tailwindcss` | 4.x | CSS-first config, @theme directive, dark: variants |
| Terminal emulator | `@xterm/xterm` | 6.x | Terminal rendering in browser |
| Terminal WebGL | `@xterm/addon-webgl` | 0.19+ | GPU-accelerated rendering, 60fps |
| Terminal search | `@xterm/addon-search` | latest | Cmd+F find in scrollback |
| Terminal fit | `@xterm/addon-fit` | latest | Auto-resize terminal to container |
| Split panes | `paneforge` | latest | Resizable panel splits (shadcn-svelte compatible) |
| Icons | `lucide-svelte` | latest | Clean developer-focused icons |

### Tauri Plugins

| Plugin | Purpose |
|---|---|
| `tauri-plugin-pty` or `portable-pty` (Rust crate) | PTY management for Claude Code child processes |
| `tauri-plugin-single-instance` | Prevent multiple app instances, focus existing |
| `tauri-plugin-deep-link` | `pantheon://` URL scheme |
| `tauri-plugin-store` | User preferences persistence (settings.json) |
| `tauri-plugin-window-state` | Auto-save/restore window position and size |
| `tauri-plugin-notification` | Background PTY input notifications |
| `tauri-plugin-dialog` | Directory picker for project selection |
| `tauri-plugin-prevent-default` | Suppress WKWebView Cmd+F, Cmd+P, etc. |

### Rust Backend (src-tauri)

| Crate | Version | Purpose |
|---|---|---|
| `tauri` | 2.x | App framework |
| `tokio` | 1.x (rt-multi-thread) | Async runtime for PTY readers, WebSocket |
| `tokio-tungstenite` | latest | WebSocket client to daemon |
| `reqwest` | 0.12.x (rustls-tls) | HTTP client for daemon REST endpoints |
| `serde` / `serde_json` | 1.x | JSON serialization |
| `shared-types` | workspace path dep | AgentStreamEvent, WorkerLifecycle types |
| `portable-pty` | 0.9+ | PTY spawning (if not using tauri-plugin-pty) |
| `sysinfo` | latest | Process scanning, Physical Footprint memory |
| `uuid` | 1.x | PANTHEON_SESSION_ID generation |
| `pidfile-rs` | latest | Daemon PID file locking |
| `nix` | latest | Signal sending (SIGTERM for kill) |
| `rand` | latest | Token generation if needed |

### Workspace Structure

```
triumvirate/
  daemon/
    Cargo.toml              ← workspace root (existing)
    crates/
      shared-types/         ← shared between daemon and Pantheon
      daemon-core/
      daemon-http/
      triumvirate/          ← daemon binary
      ...
  pantheon/
    src-tauri/
      Cargo.toml            ← workspace member of daemon/Cargo.toml
      src/
        main.rs             ← Tauri entry point
        lib.rs              ← Tauri commands, PTY mgmt, daemon client
      tauri.conf.json
      capabilities/
        default.json        ← Permissions for plugins
    src/
      App.svelte            ← Root component
      lib/
        components/
          Sidebar.svelte
          TerminalPanel.svelte
          StatusArea.svelte
          WorkerDrawer.svelte
          TokenEconomics.svelte
          FleetStatus.svelte
          SystemHealth.svelte
        stores/
          daemon.ts          ← WebSocket connection, events
          sessions.ts        ← Terminal panel state
          workers.ts         ← Worker hierarchy
          preferences.ts     ← Theme, settings
      app.css                ← Tailwind imports
    package.json
    svelte.config.js
    vite.config.ts
    tailwind.config.ts
```

### Build & Distribution

| Item | Value |
|---|---|
| Target | macOS aarch64 (Apple Silicon) |
| Distribution | Unsigned .dmg |
| Code signing | Deferred to post-v4.0 |
| Daemon bundled | Yes, in Contents/Resources/ or Contents/MacOS/ |
| CLI symlink | /usr/local/bin/pantheon via install script |
| URL scheme | pantheon:// |
| Cargo workspace | pantheon/src-tauri/ is a member of daemon/Cargo.toml workspace |
| Cargo.lock | Single lock file at daemon/ (workspace root) |

---

## v3.9.0 — Daemon Backend

### Existing Stack (unchanged)

| Component | Version |
|---|---|
| Rust edition | 2024 |
| Workspace version | 3.3.0 → 3.9.0 |
| axum | 0.8 |
| tokio | 1.x |
| rmcp | 1.3.0 |
| serde | 1.x |
| SQLite (via rusqlite or similar) | embedded |
| tracing + opentelemetry | 0.1 / 0.28 |

### New Dependencies for v3.9.0

| Crate | Purpose |
|---|---|
| `pidfile-rs` | PID file management with flock |
| No new external crates expected | Lineage fields, ring buffer, REST endpoints all use existing deps |

### New Daemon Endpoints

| Method | Path | Purpose | Response Shape |
|---|---|---|---|
| GET | /api/workers | All active sessions/workers with lineage | `{ workers: [{ id, agent, name, status, parent_session_id, root_session_id, task_id, elapsed_ms }] }` |
| GET | /api/fleet/:build_id | ABE task status for a build | `{ tasks: [{ id, status, files, elapsed_ms, worker_id }] }` |
| GET | /api/fleet | All active fleet builds | `{ builds: [{ id, task_count, completed, failed }] }` |
| GET | /api/state | Full state snapshot for reconnect | `{ sessions, workers, fleet, version }` |

### New WebSocket Events

| Event Type | Payload |
|---|---|
| WorkerLifecycle::Spawned | `{ agent, session_name, task_id, parent_session_id, root_session_id, seq }` |
| WorkerLifecycle::Completed | `{ agent, session_name, task_id, commit_sha, elapsed_ms, seq }` |
| WorkerLifecycle::Failed | `{ agent, session_name, task_id, error_message, seq }` |

### SQLite Schema Additions

```sql
ALTER TABLE sessions ADD COLUMN parent_session_id TEXT;
ALTER TABLE sessions ADD COLUMN root_session_id TEXT;
ALTER TABLE sessions ADD COLUMN pantheon_session_id TEXT;
```

---

## Testing

| Layer | Tool | Scope |
|---|---|---|
| Rust unit tests | cargo test | Daemon endpoints, PTY management, process scanning |
| Svelte component tests | vitest + @testing-library/svelte | Frontend components |
| Frontend E2E (UI only) | Playwright against Vite dev server | Sidebar, status area, theme switching |
| Native E2E (macOS) | tauri-plugin-pilot | Full app: PTY, tray, window management |
| Integration | cargo test + daemon mock | WebSocket client, REST client, event parsing |

---

## Performance Budgets

| Metric | Target |
|---|---|
| App shell memory | < 100MB Physical Footprint |
| Per terminal panel | ~60MB (WebGL disposed on hidden tabs) |
| 5 panels total | ~400MB |
| App launch | < 3 seconds to usable state |
| Terminal rendering | 60fps during streaming |
| WebSocket latency | < 100ms event display |
| Process scan interval | 5 seconds |
| Daemon health poll | 10 seconds |
| Token economics poll | 5 seconds |
| Fleet status poll | 3 seconds |
