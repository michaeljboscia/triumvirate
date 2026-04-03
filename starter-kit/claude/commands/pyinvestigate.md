---
description: Pythia Investigate — Interactive Project Search
---

# Pythia Investigate — Interactive Project Search

Search a project's indexed knowledge using Pythia's local search engine. This is NOT the Oracle daemon — this is local code/doc search via the `pythia` MCP server.

## Usage
`/pyinvestigate <question or topic>`

The user might give you:
- A precise question: "how does the narrative generator build subject lines"
- A vague topic: "correlation engine"
- A debugging question: "where does the persona mapping happen"
- A research question: "what do we know about email open rates"

All of these are valid. Pythia indexes code, docs, SQL, research files, prompts, and config — not just source code.

## How to Respond

### If the question is clear enough to search immediately:

1. Tell the user what you're searching for and which mode you're using
2. Call `mcp__pythia__lcs_investigate` with the query
3. Synthesize the results into an answer — don't just list files

**Good response:** "Pythia found the subject line logic across 3 files. Here's how it works: [explanation with file:line references]"

**Bad response:** "Here are 12 search results: 1. file.ts 2. file.ts 3. file.ts..."

The user wants understanding, not a file listing. Read the chunks Pythia returns and explain what they mean together.

### If the question is too vague:

Ask ONE clarifying question. Not three. Not a menu of options. One question.

- "Correlation engine — are you looking for how it ranks signals, or how it connects to the narrative generator?"
- "Email rules — the subject line rules, the banned phrases list, or the validation logic?"

Then search with whatever they say.

### Search Modes

Pick the right one automatically. Don't ask the user which mode they want — they don't know and don't care.

- **semantic** (default) — finds things by meaning. Use for almost everything.
- **structural** — follows import/call graph edges. Use when the user asks "what calls X" or "what depends on X". Only works for TypeScript/JavaScript.
- **reasoning** — searches captured architectural thoughts. Use when the user asks "why did we..." or "what was the decision about..."

### After showing results:

Offer exactly ONE follow-up, not a menu:

- If results were broad: "Want me to dig into [the most interesting finding] specifically?"
- If results were narrow: "Want me to search for related topics like [suggestion]?"
- If results were empty: "Nothing came up. Try rephrasing — what specifically are you trying to find?"

### If Pythia returns garbage or nothing:

Don't apologize or hedge. Say what happened and what to do:

- "Pythia didn't find anything useful for that query. The index might be stale — when did you last change those files? We can re-index with `pythia init --force`."
- "The results are all from docs, not code. Want me to search again with more specific function/class names?"

## Workspace Selection

Every tool accepts an optional `workspace` parameter. If the user specifies a workspace, pass it:
`mcp__pythia__lcs_investigate({ query: "...", intent: "semantic", workspace: "pythia" })`

If not specified, the connection default is used. If the user asks about a different project, list workspaces and ask.

If the user asks about a project that isn't indexed, tell them straight: "That project doesn't have a Pythia index. Want me to set one up? It takes about 20 minutes."

## What This Is NOT

- NOT the `/pythia` skill (that includes Oracle reasoning — this is LCS search only)
- NOT a database query (use Supabase MCP for live data)
- NOT a file reader (use Read tool to look at specific files)

This is semantic search over everything in the project directory that was indexed.
