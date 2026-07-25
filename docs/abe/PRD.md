# Autonomous Build Enforcement — Product Requirements Document

**Spec:** `/Users/you/projects/triumvirate/specs/AUTONOMOUS_BUILD_ENFORCEMENT.md`
**Date:** 2026-04-07
**Goat Rodeo:** 8 rounds, Phase 3 CLEAN

---

## Product Summary

A three-phase system that lets the human walk away after approving a spec. Stateless Codex workers execute tasks while stateful Claude (orchestrator) and Gemini (auditor) maintain project visibility. Mechanical enforcement prevents agent drift without depending on agent compliance.

---

## Features

### FEAT-001: Daemon MCP Adapter (REQ-A1.1)

**Priority:** P0 — Phase 1 prerequisite for everything
**User Story:** As Claude (orchestrator), I can dispatch tasks to Codex and query Gemini through MCP tool calls without human intermediation.

**Acceptance Criteria:**
- 7 MCP tools registered: `dispatch_codex`, `dispatch_codex_worktree`, `query_gemini`, `query_gemini_review`, `get_task_status`, `get_task_output`, `cancel_task`
- Tools coexist with existing `spawn_session`/`ask_session` tools (different lifecycle models)
- Daemon-down returns `DAEMON_UNAVAILABLE` with structured error
- Auto-restart attempted via local shell script (not MCP)

### FEAT-002: Codex Dispatch Interface (REQ-A1.2)

**Priority:** P0 — Phase 1
**User Story:** As the daemon, I atomically set up a worktree with enforcement artifacts and spawn a sandboxed Codex worker.

**Acceptance Criteria:**
- `dispatch_codex_worktree` accepts `sha`, `briefing_content` (string), `contract_fields` (JSON) as parameters
- Non-blocking — returns `task_id` immediately
- 9-step atomic setup: worktree → .triumvirate/ → briefing + contract → validate-task.sh copy → hook install → Codex spawn → build env → monitor → return
- Atomic rollback on any setup failure (worktree removed, clean error returned)
- Optional `keep_failed_worktree=true` debug flag
- Per-task timeout via `task_timeout_sec` (SIGTERM → 10s grace → SIGKILL + git lock cleanup)

### FEAT-003: Gemini Query Interface (REQ-A1.3)

**Priority:** P0 — Phase 1
**User Story:** As Claude (orchestrator), I can query Gemini for code review and failure diagnosis.

**Acceptance Criteria:**
- Review mode: receives git diff, returns structured review (concerns/suggestions/clean)
- Diagnosis mode: receives failure description, returns root cause + recommended fix
- On failure cases: also accepts briefing + contract for orchestrator error diagnosis
- Uses existing daemon → Gemini CLI pathway

### FEAT-004: contract.json Generation (REQ-A2.1)

**Priority:** P1 — Phase 2
**User Story:** As Claude (orchestrator), I generate a machine-readable enforcement contract for each task.

**Acceptance Criteria:**
- JSON schema with fields: task_id, req_ids, wave, file_policy (default-deny), allowed_files, forbidden_files, allowed_commands (token prefix arrays), forbidden_commands, commit_format, test_command, task_timeout_sec, done_when, reality_test
- Written to `.triumvirate/contract.json` by the daemon during dispatch
- `file_policy: "default-deny"` — unlisted files are blocked

### FEAT-005: Orchestrator Enforcement Hooks (REQ-A2.2)

**Priority:** P1 — Phase 2
**User Story:** As the system, Claude Code PreToolUse hooks prevent the orchestrator from violating file scope and command restrictions.

**Acceptance Criteria:**
- File scope guard: PreToolUse hook on Write/Edit reads contract.json, blocks writes to unlisted files
- Command guard: PreToolUse hook on Bash reads contract.json, checks token prefix matching
- Error messages follow Fleek pattern: state rule → show violation → provide fix
- Hooks fire in Claude Code auto mode (confirmed by research)

### FEAT-006: Worker Enforcement (Codex Sandbox + Git Hooks) (REQ-A2.3)

**Priority:** P1 — Phase 2
**User Story:** As the system, three independent mechanisms prevent Codex workers from violating their contract.

**Acceptance Criteria:**
- Codex OS sandbox (`--sandbox workspace-write`) restricts filesystem to worktree
- Git pre-commit hook (static generic script, copied per-worktree via `core.hooksPath`) validates commit format, file scope, stub markers
- Post-commit validate-task.sh checks commit + full test suite
- Red team acceptance test: deliberately non-compliant worker is blocked on all four violation types

### FEAT-007: Pre-Commit Hook (REQ-A2.4)

**Priority:** P1 — Phase 2
**User Story:** As the system, a git pre-commit hook mechanically rejects non-compliant commits.

**Acceptance Criteria:**
- Static generic script reading `.triumvirate/contract.json` at runtime
- Checks: commit message format, file scope (default-deny), stub markers, fast test command
- Error messages: state rule → show violation → provide fix code
- Installed per-worktree via `git config --worktree core.hooksPath .triumvirate/hooks/`
- Unit tests for contract parsing, file matching, error formatting BEFORE red team test

### FEAT-008: Post-Commit Validation (REQ-A2.5)

**Priority:** P1 — Phase 2
**User Story:** As the system, validate-task.sh runs after each commit and writes machine-readable results.

**Acceptance Criteria:**
- Writes results to `.triumvirate/VALIDATION_LOG.md`
- Emits machine-readable classification hints for mechanical failure classifier
- `.triumvirate/` path awareness for contract.json
- Stub patterns include: todo!(), unimplemented!(), TODO, FIXME, XXX, HACK, NotImplementedError, placeholder, throw new Error("not implemented"), `pass` as sole function body
- Exit codes: 0=PASS, 1=BLOCKED, 2=WARN

### FEAT-009: Wave Boundary Gate (REQ-A2.6)

**Priority:** P1 — Phase 2
**User Story:** As Claude (orchestrator), I validate the entire wave before proceeding to the next.

**Acceptance Criteria:**
- No-overlap invariant: static analysis confirms no two tasks in wave share allowed_files
- Collects all validation results from wave tasks
- Dispatches Gemini review of all wave code
- Runs full test suite on merged state
- Blocks next wave on any failure
- Writes wave summary to BUILD_MANIFEST.md
- Updates BUILD_STATE.json

### FEAT-010: Orchestration Loop (REQ-A3.1)

**Priority:** P2 — Phase 3
**User Story:** As Claude (orchestrator), I execute the full build loop: generate briefings, dispatch workers, monitor, validate, handle failures, write manifests.

**Acceptance Criteria:**
- Concurrent wave dispatch with configurable `max_parallel` (default: 2)
- Non-blocking dispatch, polling monitor
- 7-step per-task loop: briefing+contract → dispatch → monitor → validate → handle result → update state → context dies
- BUILD_STATE.json updated after every task
- BUILD_MANIFEST.md and DEVIATION_LOG.md append-only per task
- Resume protocol: human says "resume", Claude reads BUILD_STATE.json, reconciles with daemon

### FEAT-011: Build Artifacts (REQ-A3.2)

**Priority:** P2 — Phase 3
**User Story:** As the system, every build produces machine-readable and human-readable audit trails.

**Acceptance Criteria:**
- BUILD_STATE.json: quick-resume checkpoint with tasks_completed, tasks_running, tasks_remaining, tasks_failed, max_parallel, build_started_at, build_timeout_sec (optional)
- BUILD_MANIFEST.md: append-only markdown, one row per task with commit SHA, validation status, Gemini review, timestamp
- DEVIATION_LOG.md: every departure from plan, every failure, every Gemini concern, every "clean — no deviations" entry
- Crash recovery: detect interrupted tasks, forensic snapshot, quarantine dirty worktree, redispatch from stable SHA

### FEAT-012: Briefing Generation (REQ-A3.3)

**Priority:** P2 — Phase 3
**User Story:** As Claude (orchestrator), I generate briefings that focus the worker without embedding file contents.

**Acceptance Criteria:**
- Briefing contains: task XML, Wave 0 contracts, file read list (not contents), prior task synopsis (10-20 lines), known hazards, execution contract
- Worker reads full files from worktree filesystem — briefing is advisory
- Claude passes briefing as string parameter to dispatch (daemon writes the file)

### FEAT-013: Repair Briefing Generation (REQ-A3.4)

**Priority:** P2 — Phase 3
**User Story:** As Claude (orchestrator), I generate size-capped repair briefings for failed tasks.

**Acceptance Criteria:**
- Rigid template: What Failed → Root Cause → The Fix → Do NOT
- No raw log paste, no conversation history, diff-focused only
- Size-capped and linted before dispatch

### FEAT-014: Failure Classification (REQ-A5)

**Priority:** P2 — Phase 3
**User Story:** As the system, failures are mechanically classified by evidence, not LLM judgment.

**Acceptance Criteria:**
- worker-error: validate-task.sh found stub markers or test failures
- contract-error: pre-commit hook blocked a file the worker legitimately needs
- environment-error: subprocess exit indicates missing binary or sandbox error
- orchestrator-briefing-error: everything else (conservative default → Gemini reviews)
- Per-class retry caps: worker-error 3, contract-error 2, orchestrator-briefing-error 2, total 5

### FEAT-015: Escalation Protocol (REQ-A3.6)

**Priority:** P2 — Phase 3
**User Story:** As Claude (orchestrator), I escalate to the human with full diagnostic payload when retries are exhausted.

**Acceptance Criteria:**
- Priority order: environment-error → wave gate fail → retry cap → orchestrator-briefing-error → contract-error → worker-error → Gemini advisory
- Escalation payload includes ALL briefings Claude generated
- Daemon unreachable: one auto-restart via shell, then HALT
- Total retry cap: 5 per task across all failure classes

---

## Feature Dependency Map

```
Phase 1 (manual build):
  FEAT-001 → FEAT-002 → FEAT-003

Phase 2 (enforcement — can use Phase 1 tools):
  FEAT-004 → FEAT-005, FEAT-006, FEAT-007, FEAT-008
  FEAT-009 depends on FEAT-006, FEAT-008

Phase 3 (orchestration — requires Phase 1 + Phase 2):
  FEAT-010 → FEAT-011, FEAT-012, FEAT-013
  FEAT-014, FEAT-015 depend on FEAT-008, FEAT-010
```
