# Pattern: Codex→Gemini Delegated Review

For large codebases: Claude dispatches Codex with a task. Codex decides to spin up its own Gemini daemon to load the full codebase into a 2M-token context window. Claude gets back a complete review without ever loading the files itself.

This is the triangle topology — and why it exists.

```
Claude → Codex → Gemini
              ↑ Codex manages this autonomously
```

## When to use

- Codebase is too large for Codex's context window (>100K tokens of source)
- You want code-aware reasoning (Codex) with full context (Gemini) in one pass
- You want Claude's context window to stay completely clean

## Prerequisites

Codex must be wired to the Gemini MCP server (`docs/setup/codex.md`). Without this, Codex cannot spawn Gemini autonomously.

## How it works

### Claude's side

Claude dispatches Codex with a task and a path. That's it.

```
spawn_daemon({ cwd: "/path/to/project" })

ask_daemon({
  daemon_id: "cd_abc123",
  question: "Perform a security audit of /path/to/project/src/. For files over 200 lines, use Gemini to load them — spawn a Gemini daemon, read the file, ask targeted questions, then dismiss. Return a structured report with findings by severity."
})
```

Claude never touches the source files. Codex manages the Gemini daemon lifecycle internally.

### What Codex does (autonomously)

```
Codex receives the task
  → scans /path/to/project/src/
  → for large files:
      spawn_daemon(cwd: /path/to/project)   ← Codex spawning Gemini
      ask_daemon("Read /path/to/project/src/auth.ts and identify security issues")
      ask_daemon("Read /path/to/project/src/api.ts and check input validation")
      dismiss_daemon()
  → synthesizes findings
  → returns complete report to Claude
```

## Example: full codebase review

```
# Claude dispatches Codex
spawn_daemon({ cwd: "/path/to/large-project" })

ask_daemon({
  daemon_id: "cd_abc123",
  question: "Review /path/to/large-project for security vulnerabilities. The project is a Node.js API. Use Gemini for any file over 300 lines — spawn a daemon, read the file, ask about vulnerabilities, dismiss. Structure your report as: Critical → High → Medium → Low → Informational."
})
```

## Token flow

```
Claude context:   [task dispatch] + [final report]       ~5K tokens total
Codex context:    [task] + [file listing] + [synthesis]  ~20K tokens
Gemini context:   [full source files] + [Q&A]           ~500K+ tokens

Total cost:       pays for actual reasoning at each layer
```

Without this pattern, Claude would hold all ~500K tokens of source while also reasoning about it.

## Named Gemini sessions within Codex

If Codex needs to review the same codebase across multiple tasks, it can use named sessions:

```
# Codex instruction (in AGENTS.md or task description):
spawn_daemon({ session_name: "project-audit", cwd: "/path/to/project" })
# If session already exists → resumes instantly with zero bootstrap cost
```

This is useful when Claude dispatches multiple Codex tasks in sequence against the same large codebase.
