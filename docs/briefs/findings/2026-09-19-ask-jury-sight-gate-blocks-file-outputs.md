# ask_jury: required_sources and outputs cannot be used together

Found 2026-09-19 17:33 ET on the first real labeling call from the mneme Stacklist tree
(jury-17606642-3366-468a-ad18-9e163d591ac2, cwd `/Users/michaelboscia/projects/mneme-bosciamem/stacklist/gold/s1/wk`).

- `required_sources` implies `require_sight`, which dispatches each seat as a read-only REVIEW.
- `outputs` exists so each seat writes its answers to a file and only counts come back (sealed labels).
- Grok did exactly what it was asked, wrote its label file, and the bridge rejected the turn: "Grok was dispatched as a
  review but MODIFIED files: write, write. A reviewer looks and does not touch." The file then did not exist.

So the tool's two sealing features (prove the seat read the sources; keep the answers out of the caller's context)
exclude each other. Suggested fix: when `outputs` is set, allow writes to exactly the declared output paths and still
reject any other write. Same run, not defects in this tool but worth knowing: Grok at `fast` hit its 12-turn cap on a
two-file read-and-write task; the Gemini seat returned empty output (SUCCESS, 3 tool calls) where `agy --print`
direct worked earlier the same day; Codex was out of quota.

Side effect seen: running with that cwd created a new `.triumvirate/ledger.db` there, one more per-folder ledger
(see `wiki-usage-measurement-brief.md`, population hygiene).

## The fix is in two places, not one

1. **Detection**, `daemon/crates/triumvirate/src/agent_exec.rs:2447` `enforce_reviewer_sight`. It rejects the turn when
   any tool call is `WriteFile` or `EditFile`. Its own comment says detection is the second line of defence and cannot
   carry the check alone, because `codex-exec-json` stamps every call `Bash`.
2. **Containment**, per adapter, which is what actually blocks the write:
   `daemon/crates/mcp-bridge/src/agy.rs:192` (`read_only` forces the seatbelt on),
   `daemon/crates/mcp-bridge/src/grok.rs:526` (`--sandbox`), and the codex sandbox policy in `codex_capabilities.rs`.

So allowing a jury seat to write its own label file means letting the declared `outputs` path through the sandbox in
each adapter, then narrowing the detection to reject writes to anything else. Scope it to paths named in `outputs`
for that call, so the reviewer sandbox is unchanged everywhere else. The named-sources half of the gate needs no
change and should stay on: it is what proves the seat opened the evidence.

A smaller alternative, if the sandbox work is not wanted: when `outputs` is set, do not force review mode at all,
and keep only the named-sources check. That gives sealed labels plus proof of reading, and gives up the read-only
sandbox for jury calls only.

Not urgent for the caller that found it: the mneme Stacklist jury is blocked on Codex quota until 2026-09-20 14:03
either way, so a fix landing before then costs that caller nothing.
