# 032 — Multi-Agent Coordination Landscape (April 2026)

**Source:** Goat Rodeo Round 4 research — 12 searches across Claude, Codex, Gemini ecosystems

---

## How Claude Agent Teams Does It (Official)

| Mechanism | How It Works |
|-----------|-------------|
| **Git worktrees** | Each agent gets an isolated worktree. Same .git, separate working dirs. Prevents direct overwrites. |
| **File lock files** | Agents write lock files to `.claude/tasks/` to claim work. Prevents two agents taking same task. |
| **Shared task list** | Team lead decomposes work into subtasks with dependencies. Agents claim available tasks. Completed tasks unblock dependents. |
| **Peer-to-peer mailbox** | Agents message each other directly — share discoveries mid-task, challenge findings, coordinate. NOT routed through lead. |
| **Directory ownership** | Each agent assigned file/directory scope in the prompt. Reduces same-file collision risk. |
| **Sequential merge** | Branches merged one at a time, not simultaneously. Each merge gets full context of previous merges. |
| **Human gate for complex conflicts** | Simple conflicts (two agents add to same list) = AI resolves. Architectural conflicts = human decides. |

**What they DON'T solve:** Cross-model coordination. Agent Teams is Claude-only. No Gemini, no Codex.

---

## Community Solutions

| Tool | What It Does | Language | Status |
|------|-------------|----------|--------|
| **Event Horizon** (VS Code ext) | File-level locking across multiple AI agents (Claude, Cursor, Copilot). Plan coordination to prevent duplicate work. | JS | Active (Mar 2026) |
| **Clash** | Real-time conflict detection between git worktrees. Alerts agents before they commit conflicting changes. | Unknown | Active |
| **Ruflo** (née Claude Flow) | Rust-based orchestrator. 100+ agents in coordinated swarms. Hierarchical with shared memory. | Rust | Active |
| **Agentrooms** | Multi-agent workspace for Claude Code. Built-in orchestrator, specialized agent routing. | Unknown | Active |
| **CC Mirror** | Exposed hidden multi-agent system inside Claude Code codebase. Pure task decomposition with blocking relationships. | Unknown | Active |
| **AGOR** | AgentOrchestrator. Parallel Divergent and Pipeline coordination strategies. | Unknown | Active |
| **swarms-rs** | Rust-based enterprise multi-agent orchestration. Speed + efficiency focus. | Rust | Active |

---

## Cross-Provider Solutions (Codex/Gemini)

| Provider | Multi-agent Support | Coordination Mechanism |
|----------|-------------------|----------------------|
| **Codex** | Multiple `exec` instances, ~8 concurrent API requests | No built-in coordination. Port conflicts on local machine. Multi-session management feature requested (Nov 2025). |
| **Gemini** | Multiple CLI sessions, 1,000-2,000 RPD quota | Jules extension for async background tasks. No built-in multi-agent coordination. |
| **OpenAI Swarm** | Experimental framework (Oct 2024) | Lightweight multi-agent, chat-centric, agent handoffs. Python only. |
| **Google ADK** | Agent Development Kit (Apr 2025) | Hierarchical agent composition, Vertex AI integration. |
| **AutoGen** (Microsoft) | Multi-turn agent conversations | Chat-centric orchestration, flexible. |
| **CrewAI** | Role-based multi-agent | Structured task execution with defined roles/goals. |

---

## The Gap — What Nobody Has Built

**Cross-model fleet coordination.** Every solution above is single-provider:
- Claude Teams = Claude only
- OpenAI Swarm = OpenAI only
- Google ADK = Gemini only

NOBODY has built: "3 Claudes + 2 Geminis + 2 Codexes working on the same codebase with shared memory, real-time coordination, and unified dashboard."

That's what the Triumvirate is.

---

## Dominant Patterns (Convergent Across All Solutions)

1. **Git worktrees for isolation** — THE pattern. Every serious multi-agent tool uses this.
2. **Contract/interface definition before parallel work** — define boundaries, then fan out.
3. **Shared task list with dependency tracking** — agents claim tasks, completions unblock dependents.
4. **File-level locking** — necessary but not sufficient (shared imports break it).
5. **Sequential merge, not parallel** — one branch at a time, with full context.
6. **Lead agent decomposes, workers execute** — orchestrator-worker is universal.
7. **Peer-to-peer messaging** — Claude Teams' mailbox is the newest innovation.
