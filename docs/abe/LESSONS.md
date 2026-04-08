# Autonomous Build Enforcement — Lessons Learned

**Seeded from:** Goat Rodeo (8 rounds), Codex 10-failure post-mortem, Triumvirate v1/v2 failures

---

## From the Goat Rodeo

### Claude Code hooks don't apply to Codex
**What happened:** Original spec assumed PreToolUse hooks would guard Codex workers. Research in Round 1 proved Codex CLI has its own OS-level sandbox (Seatbelt/Landlock) and does not use Claude Code hooks.
**Rule:** Enforcement is split by agent type. Claude hooks for orchestrator. Codex sandbox + git hooks for workers. Never conflate them.

### Codex workspace-write can't read outside the worktree
**What happened:** validate-task.sh lives at `~/.claude/scripts/` — unreachable from Codex's sandbox.
**Rule:** Copy any external scripts into `.triumvirate/` during dispatch. Use `--add-dir` only if copying is impractical.

### Git worktrees use the main repo's hooks by default
**What happened:** Assumed pre-commit hooks would fire automatically in worktrees. They don't — worktrees share the parent's `.git/hooks/`.
**Rule:** Always set `git config --worktree core.hooksPath .triumvirate/hooks/` during dispatch.

### "What This Does NOT Cover" needs the same scrutiny as "What This Covers"
**What happened:** Dashboard was excluded in the original draft. 8 rounds of goat rodeo never questioned it. User caught it in Phase 4.
**Rule:** Interrogate scope exclusions as aggressively as scope inclusions. Exclusions are design decisions too.

### MCP tools can't restart the MCP server
**What happened:** Spec said "orchestrator restarts daemon via MCP." But if the daemon IS the MCP server, MCP tools are unavailable when it's down.
**Rule:** Daemon restart uses local shell script (Bash tool), not MCP.

### Context noise causes drift — stateless workers are the fix
**What happened:** Codex post-mortem showed 10 failure modes. Root cause: context accumulation buries the contract signal.
**Rule:** Workers are stateless. One session per task. Born, execute, commit, die. State lives in the orchestrator and on disk (BUILD_STATE.json, BUILD_MANIFEST.md).

## From Triumvirate v1

### Build the steering wheel before the engine
**What happened:** v1 passed 6 rounds of goat rodeo, 190 tests, 13 canonical docs — and shipped a daemon the user couldn't reach because nobody asked "how does the human call this?"
**Rule:** Phase 0 of every goat rodeo traces the human's path. If ANY hop is undefined, the spec fails.

## From Codex Post-Mortem

### Agents can't self-diagnose their own errors
**What happened:** When Claude writes a bad briefing causing worker failures, Claude classifying the failure as "worker-error" instead of "orchestrator-briefing-error" wastes retries.
**Rule:** Classification is mechanical (by evidence from validate-task.sh), not LLM judgment. Ambiguous cases go to Gemini.

### Blind retries waste attempts
**What happened:** Same worker, same briefing, same contract — failed 3 times the same way.
**Rule:** Four failure classes with different retry strategies. contract-error fixes the contract. orchestrator-briefing-error rewrites the briefing. environment-error halts immediately. Only worker-error dispatches a new worker with a repair briefing.
