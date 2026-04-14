# Pantheon v4.0 — AI Agent Instructions

**Read this file first at the start of every session.**

---

## Project

Pantheon is a native macOS app (Tauri v2 + Svelte 5 + xterm.js) for multi-agent orchestration. It replaces 17+ Terminal.app windows with one GUI showing embedded Claude Code sessions, daemon-spawned workers in a hierarchical sidebar, system memory pressure, and fleet build status.

Two releases: v3.9.0 (daemon backend) → v4.0.0 (Tauri app).

---

## Canonical Docs (Source of Truth)

| Doc | Path | What it governs |
|---|---|---|
| Spec | specs/PANTHEON_V4.md | Requirements (35 REQs) |
| PRD | docs/4.0.0/PRD.md | Features (15 FEATs) with acceptance criteria |
| App Flow | docs/4.0.0/APP_FLOW.md | Every user journey, screen state, error handling |
| Tech Stack | docs/4.0.0/TECH_STACK.md | Exact frameworks, versions, dependencies |
| Design System | docs/4.0.0/DESIGN_SYSTEM.md | Colors, typography, spacing, icons |
| Frontend Guidelines | docs/4.0.0/FRONTEND_GUIDELINES.md | Component architecture, patterns, Svelte 5 rules |
| Backend Structure | docs/4.0.0/BACKEND_STRUCTURE.md | API contracts, schema, auth |
| Implementation Plan | docs/4.0.0/IMPLEMENTATION_PLAN.md | 29 tasks, 9 waves, execution contract |
| Test Plan | docs/4.0.0/TEST_PLAN.md | Every REQ mapped to a test |

**These documents are law. Do not deviate without explicit user approval.**

---

## Tech Stack Summary

- **App framework:** Tauri v2 (NOT v1)
- **Frontend:** Svelte 5 with runes ($state, $effect, $props) — NOT Svelte 4
- **Terminal:** xterm.js 6.x with WebGL addon — NOT canvas or DOM renderer
- **Styling:** Tailwind CSS 4.x with dark: class strategy
- **PTY:** portable-pty or tauri-plugin-pty — NOT node-pty
- **Splits:** PaneForge — for terminal area only, NOT outer layout
- **Daemon communication:** REST (reqwest) + WebSocket (tokio-tungstenite) — NOT MCP
- **Shared types:** path dependency on daemon/crates/shared-types

---

## Constraints

1. **macOS only.** Apple Silicon (aarch64). No Linux, Windows, or Intel targets for v4.0.
2. **Unsigned .dmg.** No code signing for v4.0.
3. **Daemon is a detached process.** NOT a Tauri sidecar. It must survive Pantheon quitting.
4. **Physical Footprint for memory.** NOT RSS. RSS double-counts shared libraries.
5. **Process scanning uses full command-line args.** NOT just executable name. Claude may appear as `node /path/to/claude`.
6. **PANTHEON_SESSION_ID via both transports.** HTTP header (proxy) AND MCP _meta (stdio).
7. **Template icons for menubar.** Shape-based (not color-based). Auto-adapt to dark/light mode.
8. **5,000-line scrollback.** NOT 10,000. Memory budget constraint.
9. **Worker detail in drawer.** NOT inline sidebar expansion.
10. **No mobile-first.** This is a desktop app. Minimum 900x600.

---

## Workflow Orchestration

### Plan Mode Default
- Enter plan mode for ANY non-trivial task (3+ steps or architectural decisions)
- If something goes sideways, STOP and re-plan — don't keep pushing
- Write detailed specs upfront

### Subagent Strategy
- Use subagents for research, exploration, parallel analysis
- One task per subagent for focused execution
- Keep main context clean

### Self-Improvement Loop
- After ANY correction: update LESSONS.md
- Write rules that prevent the same mistake
- Review lessons at session start

### Verification Before Done
- Never mark a task complete without proving it works
- Run tests, check the reality test, demonstrate correctness
- Ask: "Would a staff engineer approve this?"

### Autonomous Bug Fixing
- Given a bug report: just fix it
- Point at logs, errors, failing tests — then resolve them
- Zero context switching required from the user

---

## Protection Rules

### No Regressions
- Before modifying any existing file, diff what exists
- Never break working functionality to implement new functionality
- Existing daemon tests must still pass after every change

### No Assumptions
- If you encounter anything not covered by documentation, STOP and ask
- Do not infer. Do not guess.
- Every undocumented decision gets escalated

### No Contract Changes Without Approval
- If a spec defines an interface and reality doesn't match, this is a BLOCKER
- STOP. Report: "The spec says X but reality is Y because Z"
- Get user decision before writing any code

### Design System Enforcement
- Before creating ANY component, check DESIGN_SYSTEM.md first
- Never invent colors, spacing values, or tokens not in the file
- Consistency is non-negotiable

---

## Task Management

1. **Plan First:** Write plan to tasks/todo.md
2. **Verify Plan:** Check with user before starting
3. **Track Progress:** Mark items complete as you go
4. **Explain Changes:** High-level summary at each step
5. **Document Results:** Add review section to tasks/todo.md
6. **Capture Lessons:** Update LESSONS.md after corrections

---

## Session Startup Sequence

1. Read this file (CLAUDE.md)
2. Read progress.txt (where is the project)
3. Read IMPLEMENTATION_PLAN.md (what phase/step is next)
4. Read LESSONS.md (what mistakes to avoid)
5. Write tasks/todo.md (plan for this session)
6. Execute
