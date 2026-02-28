# Pattern: Codex Code Review

Dispatch Codex for git-aware code review on uncommitted changes, branches, or specific commits — without loading the diff into Claude's context.

## When to use

- Pre-commit review of staged changes
- Branch review before a PR
- Security audit of a specific file or module
- Performance review with optimization suggestions

## How it works

Codex has git awareness and reads files directly from disk. Claude dispatches a task; Codex reviews; Claude gets back the findings.

```
spawn_daemon({ cwd: "/path/to/project" })

ask_daemon({
  daemon_id: "cd_abc123",
  question: "Review the staged changes in /path/to/project. Focus on: correctness, edge cases, and any security concerns. The changes are to the authentication middleware."
})

dismiss_daemon({ daemon_id: "cd_abc123" })
```

## Git-aware patterns

Codex can diff, log, and read the codebase directly:

```
# Review uncommitted changes
"Review `git diff HEAD` in /path/to/project. Look for any obvious bugs."

# Review a specific commit
"Review commit abc123 in /path/to/project. Was this change safe?"

# Review a module
"Read /path/to/project/src/payments/ (all files) and review the payment processing logic for correctness and PCI compliance risks."
```

## Structured review requests

Get consistent output by specifying a structure:

```
ask_daemon({
  daemon_id: "cd_abc123",
  question: "Review /path/to/project/src/auth.ts. Structure your response as:
  ## Critical Issues (bugs, security)
  ## Warnings (code quality, edge cases)
  ## Suggestions (optional improvements)
  ## Verdict (approve / approve with changes / needs work)"
})
```

## Using the scratchpad for large reviews

For large codebases, have Codex write findings incrementally:

```
ask_daemon({
  daemon_id: "cd_abc123",
  question: "Review each file in /path/to/project/src/api/. After each file, write your findings to the scratchpad with write_scratchpad({ topic: 'review-<filename>', cwd: '/path/to/project', daemon_id: 'cd_abc123' }). Summarize at the end."
})
```

Claude can then read scratchpad files as they arrive, even before Codex finishes.

## Codex→Gemini for very large codebases

For codebases too large for Codex's context window alone, use the triangle topology — see `codex-gemini-delegated-review.md`.
