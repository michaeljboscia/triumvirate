# Pantheon v4.0 — Lessons

---

## L-001: TUI-in-TUI is a dead end
**What happened:** Initial spec proposed wrapping Claude Code (a React+Ink TUI) inside a Ratatui TUI. Research revealed this creates escape sequence conflicts, alternate screen buffer fights, and resize coordination between two independent layout engines.
**Rule:** Never embed one TUI application inside another TUI application. Use a proper terminal emulator (xterm.js) in a native GUI (Tauri) instead.

## L-002: Terminal multiplexers don't solve the problem
**What happened:** Tried aoe (Agent of Empires) — a tmux-based session manager. It requires manual `aoe add` for each session, creates terminal-in-terminal UX, uses arcane keybindings, and can't see daemon-spawned workers.
**Rule:** The user doesn't want another terminal. They want a native app. tmux is infrastructure, not product.

## L-003: RSS is the wrong memory metric on macOS
**What happened:** Spec initially said "RSS memory." Research revealed RSS double-counts shared libraries across processes. macOS Physical Footprint (via `footprint` or `vmmap --summary`) is what Activity Monitor shows and what the kernel uses for memory pressure decisions.
**Rule:** Always use Physical Footprint on macOS. Never report RSS as "memory used."

## L-004: Process scanning needs full command-line args
**What happened:** Spec said "scan for processes whose command starts with `claude`." Live `ps` output showed Claude Code appears as `claude --resume <session_id>` on this machine, but may appear as `node /path/to/claude` or `bun /path/to/claude` depending on installation method.
**Rule:** Read full command-line arguments via `KERN_PROCARGS2` or `sysinfo`. Match against both executable basename AND script path in argv.

## L-005: Tauri sidecars die with the parent app
**What happened:** Twins caught that REQ-028 said "bundle daemon as Tauri sidecar" but REQ-023 said "daemon survives quitting Pantheon." Tauri sidecars are child processes killed on app exit.
**Rule:** If a process must outlive the app, spawn it as a detached process using `Command::new().spawn()` with OS-level daemonization, NOT Tauri's sidecar API.

## L-006: MCP _meta field is the standard way to pass custom session IDs
**What happened:** Needed to link Pantheon terminal panels to daemon sessions. MCP spec has a `_meta` property in `initialize` params that accepts arbitrary JSON. This is the standard extension mechanism.
**Rule:** Use `_meta` in MCP initialize for custom metadata. Don't invent custom protocols. For HTTP transport, use custom headers (X-Pantheon-Session-Id).

## L-007: Goatrodeo twin reviews are MANDATORY
**What happened:** Claude skipped twin reviews in 5 of 7 rounds, auto-resolving 36 items without independent validation. When twins finally reviewed, they found 8 issues including 3 HIGH severity (sidecar lifecycle, process scanning, missing background REQ).
**Rule:** Never waive twin review without explicit user authorization. The twins exist to catch what Claude misses. The goatrodeo skill now has a HARD GATE enforcing this.

## L-008: Goatrodeo research must use live searches
**What happened:** Claude answered interrogator questions from training data in Round 2, calling it "research." User caught it. When live Gemini searches were fired, two answers changed (WebSocket filtering best practice, macOS menubar icon patterns).
**Rule:** Every goatrodeo Step 3 question with an external dimension must go through mcp__gemini__gemini-search. Training data is stale. Live data is truth.
