# 035 — Clash Source Analysis (Phase 0.2)

**Date:** 2026-04-05  
**Repo:** `https://github.com/clash-sh/clash`  
**Local clone:** `/Users/you/projects/triumvirate/.phase0_sources/clash`  
**Commit analyzed:** `2ac931c`  
**License:** MIT (`/Users/you/projects/triumvirate/.phase0_sources/clash/LICENSE`)  
**FEAT targets:** FEAT-019

---

## Scope
Clash is directly relevant to Triumvirate fleet collision prevention: worktree discovery, pairwise conflict detection, pre-write checks, and realtime watch updates.

## Key Source Files Reviewed
- `/Users/you/projects/triumvirate/.phase0_sources/clash/src/worktree/manager.rs`
- `/Users/you/projects/triumvirate/.phase0_sources/clash/src/worktree/conflict.rs`
- `/Users/you/projects/triumvirate/.phase0_sources/clash/src/check.rs`
- `/Users/you/projects/triumvirate/.phase0_sources/clash/src/watch/watcher.rs`

## Patterns Worth Borrowing
1. Worktree discovery from arbitrary path
- `WorktreeManager::discover_from(path)` resolves repo/worktree context even from file paths.
- This is ideal for per-task worktree assignment and pre-merge checks in fleet mode.

2. Read-only conflict detection via merge simulation
- `conflicts_with()` computes merge base, merges trees, and extracts conflicting paths.
- This gives conflict prediction before merge without mutating git state.

3. Pairwise conflict matrix
- `check_all_conflicts()` computes all `(i, j)` worktree pairs and captures errors per pair.
- Perfect fit for dashboard conflict heatmap and merge sequencing decisions.

4. Hook-safe machine-readable output
- `check.rs` emits structured JSON and specific exit behavior for pre-tool hooks.
- Direct fit for daemon-side guardrails before accepting fleet file writes.

5. Watch-mode filtering discipline
- Uses `.gitignore` + event-kind filtering to avoid noisy conflict recomputation.
- Useful for low-overhead realtime fleet conflict status.

## Patterns to Avoid
1. Tight CLI/hook coupling
- Clash is CLI-first with hook semantics.
- Triumvirate should internalize this logic in daemon workflows, not depend on shell-level hooks.

2. Panic paths in non-test logic
- `expect` appears in some serialization paths.
- In daemon code we should return typed errors all the way to WebSocket/REST responses.

## Triumvirate Adaptation Plan
- FEAT-019 (worktrees)
  - Implement `WorktreeManager` equivalent in Rust daemon module.
  - Compute pairwise conflict matrix and expose via `/api/fleet/status` + WS events.

- FEAT-021 (sequential merge)
  - Run pre-merge conflict simulation against next candidate branch before each merge step.
  - If risk > 0, block auto-merge and surface conflict artifact to dashboard.

- FEAT-020 (task routing)
  - Before assigning a task to an agent/worktree, evaluate likely overlap with active tasks by path set intersection + simulated merge.

## Attribution Guidance (for inline code comments)
- `// Adapted from Clash worktree discovery/conflict simulation (clash-sh/clash, MIT)`
- `// Adapted from Clash pre-edit conflict guard pattern (clash-sh/clash, MIT)`

## Decision
Clash provides the strongest concrete prior art for Triumvirate's fleet collision-prevention layer and should directly inform FEAT-019 and FEAT-021 implementation details.
