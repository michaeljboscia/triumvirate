# Autonomous Build Enforcement — Agent Instructions

**Version:** Triumvirate v3.0
**This file supplements the project CLAUDE.md — it does NOT replace it.**

---

## Canonical Documents (Source of Truth)

| Document | Path | What it governs |
|----------|------|----------------|
| Spec | `specs/AUTONOMOUS_BUILD_ENFORCEMENT.md` | Architecture, requirements, decisions (8-round goat rodeo) |
| PRD | `docs/abe/PRD.md` | Features with IDs (FEAT-001 through FEAT-015) |
| App Flow | `docs/abe/APP_FLOW.md` | Orchestration loop, entry points, error states |
| Tech Stack | `docs/abe/TECH_STACK.md` | Exact technologies, versions, costs |
| Backend Structure | `docs/abe/BACKEND_STRUCTURE.md` | MCP tool schemas, daemon architecture, pre-commit hook logic |
| Implementation Plan | `docs/abe/IMPLEMENTATION_PLAN.md` | 22 tasks, 6 waves, execution contract |
| Test Plan | `docs/abe/TEST_PLAN.md` | REQ-to-test matrix, 22 tests across 3 categories |

If it's not in the docs, it doesn't exist. If the docs conflict with a suggestion, the docs win.

---

## ABE-Specific Rules

### Enforcement Stack Is Two Systems

Claude Code PreToolUse hooks protect the ORCHESTRATOR (Claude). Codex sandbox + git hooks protect WORKERS. These are different enforcement systems. Do not confuse them.

- Claude hooks: `~/.claude/hooks/enforce-file-scope.sh`, `enforce-command-scope.sh`
- Worker hooks: `.triumvirate/hooks/pre-commit` (static generic script, reads contract.json at runtime)
- Worker sandbox: Codex `--sandbox workspace-write` (OS-level, non-bypassable)

### .triumvirate/ Is the Runtime Directory

All enforcement artifacts go in `.triumvirate/` inside the worktree:
- `BRIEFING.md` — written by daemon during dispatch
- `contract.json` — written by daemon during dispatch
- `validate-task.sh` — copied from `~/.claude/scripts/`
- `hooks/pre-commit` — copied from daemon assets
- `VALIDATION_LOG.md` — written by validate-task.sh post-commit
- `target/<task_id>/` — build artifact isolation

`.triumvirate/` is in `.git/info/exclude`. Pre-commit hook explicitly ignores `.triumvirate/*` files. Never commit anything from `.triumvirate/`.

### Default-Deny File Policy

If a file is not in `allowed_files` in contract.json, it is BLOCKED. No implicit allows. No exceptions. This is enforced by the pre-commit hook and the Codex sandbox.

### Failure Classification Is Mechanical

Do not use LLM judgment to classify failures. Read the validate-task.sh output:
- Stub markers or test failures in worker code → `worker-error`
- Pre-commit hook blocked a needed file → `contract-error`
- Missing binary or sandbox error → `environment-error`
- Everything else → `orchestrator-briefing-error` (send to Gemini for review)

### Retry Caps Are Hard Limits

- `worker-error`: max 3 per task
- `contract-error`: max 2 per task
- `orchestrator-briefing-error`: max 2 per task
- `environment-error`: 0 retries — HALT immediately
- Total across all classes: max 5 per task

On cap breach → ESCALATE TO HUMAN with all briefings attached.

### Briefings Are Advisory, Contracts Are Enforcement

BRIEFING.md tells the worker what to focus on. contract.json mechanically prevents violations. The worker CAN ignore the briefing. The worker CANNOT violate the contract.

### Workers Read Full Files

Briefings tell workers WHICH files to read first. They do NOT embed file contents. Workers read full files from the worktree filesystem.

### Dispatch Boundary

Claude generates briefing content (string) and contract fields (JSON). Claude does NOT write files to the worktree. Claude passes both as structured parameters to `dispatch_codex_worktree`. The daemon writes the files atomically.

### Gemini Visibility

- On PASS: Gemini reviews the diff BLIND (no briefing — preserving independence)
- On FAIL: Gemini gets the diff + briefing + contract (to diagnose orchestrator errors)

### BUILD_STATE.json Is the Resume File

If the session dies, the human says "resume." Read BUILD_STATE.json. Call get_task_status for each task in tasks_running to reconcile with the daemon. Continue at the first incomplete task.

---

## Session Startup Sequence (ABE Work)

1. Read this file (CLAUDE.md)
2. Read `docs/abe/progress.txt` — where is the build right now?
3. Read `docs/abe/IMPLEMENTATION_PLAN.md` — which wave/task is next?
4. Read `docs/abe/LESSONS.md` — what mistakes to avoid?
5. If BUILD_STATE.json exists — is there an active build to resume?
6. Plan your session's work in tasks/todo.md
7. Verify plan with user before executing

---

## Protection Rules

### No Regressions
Before modifying any existing file, diff what exists against what you're changing. Never break working functionality. If a change touches more than one system, verify each still works.

### No Assumptions
If you encounter anything not covered by the canonical docs, STOP and ask. Do not infer. Do not guess. Every undocumented decision gets escalated.

### No Contract Changes Without Approval
If reality doesn't match the spec (missing dependency, incompatible API), this is a BLOCKER. STOP. Tell the user: "The spec says X but reality is Y because Z." Research alternatives. Present options. Get approval before writing code.

### Scope Discipline
If you WANT to touch code outside your task's `<files>` but don't NEED to — don't. Scope discipline > local improvement. If you NEED to, use the collateral fix protocol.

---

## Verification Before Done

Never mark a task complete without:
1. `<verify>` passes
2. `<reality_test>` passes
3. `<done_when>` condition is met
4. Full test suite passes (`npm test`)
5. Git commit with task ID in message
6. validate-task.sh returns PASS or WARN (not BLOCKED)
