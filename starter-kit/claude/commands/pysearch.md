# Pythia Quick Search

Fast semantic search across a project's indexed knowledge. Returns an answer, not a file list.

This is NOT the `/pythia` Oracle daemon. This calls the `pythia` MCP server for local code/doc search.

## Usage
`/pysearch <query>`

## Instructions

1. Call `mcp__pythia__lcs_investigate` with:
   - `query`: the user's search terms exactly as typed
   - `intent`: `"semantic"`
   - `limit`: `12`

2. Read the returned chunks and synthesize a direct answer:
   - Lead with what you learned, not a list of files
   - Reference file paths and line numbers inline
   - If multiple chunks describe the same thing, combine them into one explanation
   - If the results don't answer the query, say so and suggest better search terms

3. Keep it short. This is a quick search, not an investigation. If the user wants to go deeper, they'll use `/pyinvestigate`.

## Example

User: `/pysearch banned email phrases`

Good: "The banned phrases list is in `supabase/functions/generate-narrative/v70/email-generator.ts:45`. It blocks: 'circling back', 'touching base', 'just checking in'. The CAPEL validator checks each generated email against this list and rejects any match."

Bad: "Here are 12 results from Pythia: 1. email-generator.ts 2. SYSTEM-PROMPT-v70.md 3. ..."

## Workspace Selection

Pass optional `workspace` param to target a specific workspace:
`mcp__pythia__lcs_investigate({ query: "...", intent: "semantic", workspace: "pythia" })`

If not specified, the connection default is used.

## What This Is NOT
- NOT the `/pythia` skill (that includes Oracle reasoning — this is LCS search only)
- NOT a database query
- This is local semantic search over indexed project files
