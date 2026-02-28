# Pattern: Gemini Deep Research

Dispatch multi-source research tasks to Gemini and get back a synthesized report. Gemini does the web crawling; Claude synthesizes and acts.

## When to use

- Research requiring 5+ sources
- Competitive analysis, technology evaluation, regulatory research
- Anything where you want a structured report, not just search snippets

## How it works

Two options depending on your setup:

### Option A: Via daemon (recommended for codebase-aware research)

Spawn a Gemini daemon, ask it to research a topic. Gemini's 2M context window can hold the research results AND your codebase simultaneously.

```
spawn_daemon({ session_name: "research-auth-options", cwd: "/path/to/project" })

ask_daemon({
  daemon_id: "gd_abc123",
  question: "Research OAuth2 vs API key authentication for B2B SaaS. Consider: security trade-offs, developer experience, and implementation complexity. Then read /path/to/project/src/auth/ and tell me which approach fits better given what we already have."
})

dismiss_daemon({ daemon_id: "gd_abc123" })
```

### Option B: Via Gemini native MCP (pure web research, no codebase)

If your Gemini native MCP server is configured:

```
mcp__gemini__gemini-deep-research({
  query: "OAuth2 vs API key authentication for B2B SaaS 2026"
})
```

This kicks off an async deep research job. Poll with `mcp__gemini__gemini-check-research` until complete.

## Token economics

Gemini holds the research output (often 10K-50K tokens). Claude only receives the synthesized answer. Without the daemon, that content would consume Claude's context window directly.

## Named sessions for ongoing research

If your research spans multiple days or sessions:

```
# First session
spawn_daemon({ session_name: "competitor-analysis" })
ask_daemon({ ..., question: "Research CompanyA's pricing and features." })
dismiss_daemon({ ... })  # soft dismiss — session preserved

# Next session (zero token cost to resume)
spawn_daemon({ session_name: "competitor-analysis" })
# Gemini remembers CompanyA — just ask about CompanyB
ask_daemon({ ..., question: "Now do the same for CompanyB." })
```

## Example output structure

Ask Gemini to structure its output for easy parsing:

```
ask_daemon({
  daemon_id: "...",
  question: "Research X. Structure your response as:
  ## Summary (3 bullets)
  ## Key Findings
  ## Recommendation
  ## Sources"
})
```
