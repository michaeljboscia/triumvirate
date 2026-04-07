# Autonomous Build Enforcement — Tech Stack

**Version:** Triumvirate v3.0
**Spec:** `specs/AUTONOMOUS_BUILD_ENFORCEMENT.md`

---

## Runtime Environment

| Component | Technology | Version | Notes |
|-----------|-----------|---------|-------|
| **Host OS** | macOS (Darwin) | 25.3.0 | Seatbelt sandbox for Codex workers |
| **Node.js** | Node.js | >=18.x | Daemon runtime + MCP server |
| **TypeScript** | TypeScript | >=5.x | Daemon source language (Phase 1) |
| **Shell** | zsh / bash | system | Hooks, validate-task.sh, dispatch scripts |
| **Git** | git | >=2.40 | Worktrees, hooks, pre-commit enforcement |

## Agent Stack

| Agent | Tool | Model | Role |
|-------|------|-------|------|
| **Claude** | Claude Code CLI | Opus 4.6 (1M context) | Orchestrator — generates briefings, monitors tasks, writes manifests |
| **Codex** | Codex CLI | GPT-5.2-Codex (258K-400K context) | Worker — stateless, one session per task, sandboxed |
| **Gemini** | Gemini CLI | Gemini Pro (2M context) | Auditor — blind code review on pass, full context on failure |

## Daemon Components

| Component | Location | Language | Purpose |
|-----------|----------|----------|---------|
| **Daemon core** | `daemon/` | TypeScript (Rust in v3.1) | Agent pool, message fabric, fleet coordinator, dispatch logic |
| **MCP server** | `mcp-server/` | TypeScript | stdio MCP bridge — registers tools, routes to daemon HTTP |
| **Daemon HTTP** | `daemon/` | TypeScript | localhost:8080, bearer token auth (`TRIUMVIRATE_DAEMON_TOKEN`) |

## Enforcement Stack

| Layer | Technology | Where it runs | What it enforces |
|-------|-----------|---------------|-----------------|
| **Codex OS sandbox** | Seatbelt (macOS) / Landlock (Linux) | Codex subprocess | Filesystem write scope (OS-level, non-bypassable) |
| **Git pre-commit hook** | Bash script | Git hook in worktree | Commit message format, file scope, stub markers |
| **validate-task.sh** | Bash script | Post-commit in worktree | Same as pre-commit + full test suite + VALIDATION_LOG.md output |
| **Claude Code hooks** | PreToolUse handlers | Orchestrator session | File scope guard + command guard for Claude (not Codex) |
| **contract.json** | JSON file | `.triumvirate/` in worktree | Machine-readable contract consumed by all enforcement layers |

## Build Artifacts

| Artifact | Format | Written by | Consumed by |
|----------|--------|-----------|-------------|
| **BUILD_STATE.json** | JSON | Orchestrator (Claude) | Resume protocol, crash recovery |
| **BUILD_MANIFEST.md** | Markdown (append-only) | Orchestrator (Claude) | Human review, /postrodeo |
| **DEVIATION_LOG.md** | Markdown (append-only) | Orchestrator (Claude) | Human review, /postrodeo |
| **AFTER_ACTION.md** | Markdown (per-task) | Orchestrator (Claude) | Next task briefing context |
| **VALIDATION_LOG.md** | Markdown | validate-task.sh | Orchestrator failure classification |
| **contract.json** | JSON (per-task) | Daemon (from orchestrator params) | Pre-commit hook, validate-task.sh, sandbox config |
| **BRIEFING.md** | Markdown (per-task) | Daemon (from orchestrator params) | Codex worker |

## Communication

| Path | Transport | Auth | Direction |
|------|-----------|------|-----------|
| Claude → Daemon | MCP (stdio) | Process-local (no network) | Bidirectional |
| Daemon → Codex | Subprocess (codex CLI) | N/A (child process) | Daemon spawns Codex |
| Daemon → Gemini | CLI subprocess | Gemini CLI config | Daemon invokes Gemini CLI |
| Daemon ↔ Dashboard | WebSocket | Bearer token | Broadcast state changes |

## File System Layout (per-worktree)

```
<worktree-root>/
├── src/                          # Project source (read/write per allowed_files)
├── .git/                         # Worktree git dir (read-only for Codex)
│   └── info/
│       └── exclude               # Contains: .triumvirate/
└── .triumvirate/                 # Runtime artifacts (git-ignored)
    ├── BRIEFING.md               # Task briefing (written by daemon)
    ├── contract.json             # Enforcement contract (written by daemon)
    ├── validate-task.sh          # Copied from ~/.claude/scripts/
    ├── VALIDATION_LOG.md         # Written by validate-task.sh post-commit
    ├── interrupted.patch         # Forensic snapshot on crash (if applicable)
    ├── hooks/
    │   └── pre-commit            # Static generic hook (copied by daemon)
    └── target/
        └── <task_id>/            # Isolated build artifacts (CARGO_TARGET_DIR equivalent)
```

## Dependencies

| Dependency | Purpose | Required by |
|-----------|---------|-------------|
| `codex` CLI | Worker agent | FEAT-002 |
| `gemini` CLI | Auditor queries | FEAT-003 |
| `claude` CLI | Orchestrator | FEAT-010 |
| `jq` | JSON parsing in bash hooks | FEAT-007 |
| `git` >= 2.40 | Worktree support, `core.hooksPath` | FEAT-002, FEAT-007 |
| Node.js >= 18 | Daemon runtime | FEAT-001 |

## Cost Model

| Resource | Cost | Notes |
|----------|------|-------|
| Claude Code (orchestrator) | Included in Max subscription | 5-hour rolling window, ~40-800 prompts |
| Codex CLI (workers) | Included in subscription | Per-task sessions, ephemeral |
| Gemini CLI (auditor) | $0 (CLI uses Ultra subscription) | Not SDK — no per-token cost |
| Compute | Local machine (30GB RAM, 16 CPU) | All local, no cloud infra |
