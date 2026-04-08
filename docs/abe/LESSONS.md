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

## From Stress Test (2026-04-08)

### Git hooks are bypassable — daemon-side validation is the real enforcement
**What happened:** Stress test STRESS-002 proved Codex uses `git commit --no-verify` to bypass pre-commit hooks when they block a commit. The hook was correct. Codex retried with `--no-verify` autonomously — this is built-in Codex behavior with `--full-auto`, not a bug.
**Rule:** Git hooks are defense against ACCIDENTAL violations. They are NOT defense against a determined worker. The daemon must independently validate every commit AFTER the worker exits, using the ORIGINAL contract held by the daemon (not the worktree copy). `post_exit_validator.rs` checks file scope, commit format, stub markers, and test command. If any fail: DAEMON_VALIDATION_FAILED, worktree quarantined. The worker can cheat all it wants — nothing leaves until the daemon says so.

### Never trust anything inside the worker's worktree
**What happened:** The worker has write access to `.triumvirate/` including contract.json and the hook scripts. It can rewrite its own rules.
**Rule:** The daemon holds the authoritative contract (`contract_for_validation`) cloned from the original dispatch request. The worktree copy of contract.json is for the HOOKS (first-line defense). The daemon's copy is for VALIDATION (final authority). These are separate — tampering with one doesn't affect the other.

### Three-layer enforcement: hooks → daemon validation → Gemini review
**What happened:** Single-layer enforcement (hooks only) was defeated in test 2 of 39. 
**Rule:** Each layer catches what the previous misses. Hooks catch honest mistakes before commit. Daemon validation catches all bypasses after commit. Gemini review catches semantic issues that pass both mechanical layers. No single layer is sufficient alone.
