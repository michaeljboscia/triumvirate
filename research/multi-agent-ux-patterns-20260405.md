# Multi-Agent CLI UX Patterns Research

**Date:** 2026-04-05
**Purpose:** UX pattern extraction for Triumvirate v2

---

## 1. AgentRoom (liuyixin-louis/agentroom)

**What the user types:**
- `cass index --full` to build search index
- `npm run tauri dev` to launch desktop app
- Natural language in Claude Code: "find my session about authentication"
- `cass search "auth error"` for CLI search

**How progress/status is shown:**
- Pixel-art animated characters at desks -- typing when writing code, reading when searching files, idle when waiting
- Active agents in "Work Room," idle agents walk to "Break Room"
- Speech bubbles when agent needs input or permission approval
- Sound chime when agent finishes its turn
- Token usage dashboard with real-time spend tracking
- Per-project office view to filter by project

**Failure communication:** Speech bubbles for permission denials. No explicit error UI documented -- the visual metaphor handles state transitions (working/idle/waiting).

**Issues/complaints:** No open issues found (new repo). The Windows port fork (NooRotic/agentroom-win) suggests cross-platform gaps.

---

## 2. CC-Mirror (numman-ali/cc-mirror)

**What the user types:**
- `npx cc-mirror quick --provider mirror --name mclaude` (one-shot)
- `npx cc-mirror` (interactive TUI wizard)
- `mclaude` to launch variant (each variant becomes its own CLI command)
- `npx cc-mirror doctor` for health check

**How progress/status is shown:**
- Interactive TUI with provider selection
- Each variant is fully isolated with its own config directory
- Provider-specific color themes (teal for Kimi, coral for MiniMax, etc.)

**Failure communication:** `doctor` command for health checks. No built-in orchestration status -- it's a variant manager, not an orchestrator.

**User complaints (from issues):**
- **#34**: "orchestration gets hung waiting indefinitely for subagent to finish, even when task complete" -- user must hit ESC manually to bypass. Core visibility problem: no timeout, no progress indicator, no way to know if a subagent is stuck vs working.
- **#29**: "Unable to compact" -- after orchestrator runs out of context, compact fails silently. User is stuck with no recovery path.
- **#35**: "Cannot Enter PLAN MODE" -- Shift+Tab crashes all variants. Zero error messaging.
- **#12**: Path aliasing didn't work on fresh Android/Termux install. Setup instructions assumed `.bashrc` existed.

---

## 3. Agent-Orchestrator (enigmatic-figure/agent-orchestrator)

**What the user types:**
- `python3 run_orchestrator.py` (run full plan)
- `python3 run_orchestrator.py --task-id T-001` (single task)
- `python3 run_orchestrator.py --test` (test mode)
- `python3 run_visual_editor.py` (GUI workflow designer)

**How progress/status is shown:**
- Structured JSONL logging to console and file
- Visual node editor (NodeGraphQt) for workflow design
- JSON-RPC 2.0 protocol with framed messages over stdio
- Heartbeat-based health monitoring

**Failure communication:** Retry with exponential backoff, configurable failure behavior per step, DLQ-style error handling. Logs are machine-readable JSONL.

---

## 4. Broader UX Patterns from Industry

### What users actually want (consolidated from Osmani, RedMonk, cc-mirror issues):

**P1 -- "Is it stuck?"** The #1 complaint across all tools. Users cannot distinguish between an agent that is working, waiting, or hung. CC-Mirror #34 is the canonical example. AgentRoom solves this with animation state (typing/reading/idle). CLI tools have no equivalent.

**P2 -- "What did it cost?"** Token cost visibility is now expected as baseline (RedMonk Dec 2025). AgentRoom includes a real-time spend dashboard. Most CLI orchestrators ignore this entirely.

**P3 -- "Let me intervene."** When orchestration hangs, users need an escape hatch. CC-Mirror users resort to ESC. There is no graceful "cancel this agent, keep the others" pattern in any tool reviewed.

**P4 -- "What happened while I was gone?"** Async/background agents need a catch-up mechanism. VS Code 1.107 added session lists for background agents. CLI tools have nothing -- you come back to a wall of scrolled text.

**P5 -- "One command to start."** Every successful tool has a single entry point. `mclaude`, `cass tui`, `python3 run_orchestrator.py`. The fewer decisions before first output, the higher adoption.

### Osmani's Key Framework (Conductor -> Orchestrator):

- **Conductor pattern:** Human directs one agent at a time. Effort is synchronous.
- **Orchestrator pattern:** Human front-loads specs, back-loads review. Agents work in parallel. 3 focused agents > 1 generalist agent working 3x longer.
- **Ralph Loop:** Atomic tasks in stateless iterations -- pick task, implement, validate, commit, reset context, repeat.
- **AGENTS.md:** Compound learning file that accumulates patterns across sessions.

---

## Concrete UX Patterns for Triumvirate v2

| Pattern | Implementation |
|---------|---------------|
| Heartbeat indicator | Each agent emits periodic status; orchestrator shows "alive / working / waiting / stuck" |
| Cost ticker | Running token count per agent, total spend, projected cost |
| Graceful cancel | Kill one agent without affecting siblings; reassign its work |
| Async catch-up | Structured summary of what happened since last human interaction |
| Single entry point | One command starts the fleet; zero config for happy path |
| Escape hatch | ESC or Ctrl+C triggers graceful shutdown with state preservation |
| Speech bubble / status line | One-line status per agent visible at all times (like tmux status bar) |

---

## Sources

- [AgentRoom](https://github.com/liuyixin-louis/agentroom)
- [CC-Mirror](https://github.com/numman-ali/cc-mirror)
- [Agent-Orchestrator](https://github.com/enigmatic-figure/agent-orchestrator)
- [Osmani: The Code Agent Orchestra](https://addyosmani.com/blog/code-agent-orchestra/)
- [Osmani: Conductors to Orchestrators](https://addyosmani.com/blog/future-agentic-coding/)
- [RedMonk: 10 Things Developers Want from Agentic IDEs](https://redmonk.com/kholterhoff/2025/12/22/10-things-developers-want-from-their-agentic-ides-in-2025/)
- [VS Code 1.107 Multi-Agent Update](https://visualstudiomagazine.com/articles/2025/12/12/vs-code-1-107-november-2025-update-expands-multi-agent-orchestration-model-management.aspx)
- [Orcha Multi-Agent Desktop Tool](https://medium.com/@muktharvortegix/why-multi-agent-orchestration-is-the-future-of-development-and-how-orcha-gets-you-there-c23668d51729)

---

## 5. swarms-rs (The-Swarm-Corporation/swarms-rs)

**What the user types:** Nothing -- no CLI. Users write async Rust code with `cargo add swarms-rs`, `#[tokio::main]`, `.run()` on agent builders.

**How progress/status is shown:** `RUST_LOG=debug` / `SWARMS_LOG_LEVEL=DEBUG` env vars through `tracing_subscriber`. State persistence via `.enable_autosave()` to directories. No query/inspection tool to read saved state.

**Failure communication:** Rust `Result<>` types, `.unwrap()` in all examples. No dashboard, no TUI, no status command.

**Assessment:** Pure framework, zero user-facing observability. The UX gap is total.

---

## 6. clash-sh/clash

**What the user types:**
- `clash check src/main.rs` -- single file conflict check
- `clash status` -- conflict matrix across all worktrees
- `clash status --json` -- machine-readable output for scripts/agents
- `clash watch` -- live TUI monitoring with auto-refresh
- `curl -fsSL https://clash.sh/install.sh | sh` -- install
- `claude plugin install clash@clash-sh` -- Claude Code plugin

**How progress/status is shown:**
- Conflict matrix TUI (ratatui) showing all worktree pairs with conflict counts
- Real-time `clash watch` mode with filesystem notifications (notify crate)
- PreToolUse hook intercepts Write/Edit/MultiEdit and prompts before conflicting edits
- Exit codes: 0=clean, 2=conflicts, 1=error

**Failure communication:**
- Hook fires on file writes, surfaces "ask" decision prompt
- JSON output with structured conflict details per worktree pair
- CLAUDE.md instructions for agents without hook support

**Key limitation:** Claude Code does not display `permissionDecisionReason` in prompt UI (anthropics/claude-code#24059), so conflict details are invisible in the hook flow -- user must run `clash check` manually.

**Assessment:** Best-in-class UX for its narrow problem. Solves file-level conflict detection but NOT agent liveness, progress, or coordination. Rust, single binary, zero runtime dependencies.

---

## 7. GitHub Issues: Multi-Agent Visibility Pain Points

### Ranked by community signal (upvotes + comment engagement):

| Issue | Signal | Core Pain |
|-------|--------|-----------|
| codex #2109 "Event Hooks" | 523 +1 | No hook system at all -- cannot gate, inspect, or react to agent behavior |
| codex #2604 "Subagent Support" | 318 +1 | No subagent orchestration, no child agent config |
| codex #3962 "Play a sound when done" | 131 +1 | Cannot tell when agent finishes |
| superpowers #429 "Agent Teams support" | 80 +1 | Orchestration tools exist but no skill uses them |
| codex #11701 "Subagent config" | 48 +1 | No model/effort config for child agents |
| cc #1770 "Parent-Child Agent Monitoring" | 21 +1 | Cannot see sub-agent state, cannot send corrections |
| cc #3013 "Parallel Agent Execution" | 10 +1 | No native parallel execution with visibility |
| cc #23620 "Team lost on compaction" | 9 +1 | Multi-agent state evaporates silently |
| cc #24798 "Inter-session communication" | 8 +1 | Sessions siloed, no dependency sequencing |
| cc #32650 "Completion-integrity failures" | 3 +1, 18c | 16 failure classes: agents claim done when not done |
| cc #20236 "TaskOutput hangs" | -- | Agent finishes but parent session hangs silently |

### The Three Unsolved UX Problems (updated synthesis):

**P1 -- Liveness:** "Is my agent alive or stuck?" No heartbeat, no progress indicator, no timeout detection. Users stare at terminals hoping for output. (codex #3962, cc #20236, cc-mirror #34)

**P2 -- Coordination Visibility:** "What are my N agents each doing right now?" No dashboard, no cross-session awareness. Clash solves only file-conflict subset. (cc #1770, #24798, #29086)

**P3 -- Completion Integrity:** "Did it actually finish, or just say it finished?" False completion is the most insidious failure. Agents report success with stubs, skipped tests, partial implementations. (cc #32650, superpowers verification-before-completion skill exists as a workaround)
