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
