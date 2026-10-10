# Sight gate: keep the first answer, reject less

**Working Directory:** /Users/michaelboscia/projects/triumvirate
**Git Branch:** feat/sight-salvage-main (worktree /Users/michaelboscia/projects/triumvirate-sight-salvage), based on origin/main 8d147d8
**Status:** Part 1 and most of 2a built; see "Correction" and "What was built" below

## Correction, 2026-10-10

The first draft of this brief and its first four commits were made on feat/wiki-adoption, 138 commits behind main. main had already fixed two of the buckets below: D-028 unions read_file windows, D-032 credits only the lines a read RETURNED (and sizes shell reads against a 30,000-byte output budget), and D-036 counts `;`-joined sed windows on one file. The rejection rows below predate those fixes. The work was redone on a branch from origin/main; what follows "What was built" is current.

## What was built (feat/sight-salvage-main)

- 9ad3299 D-055: a rejection saves the whole turn (reply and tool calls) to `~/.triumvirate/rejected/<id>-<uuid>.json`.
- 6bbce82 D-056a: codex's adjacent-quote rendering of a `-c` script is joined.
- 68f3fe2 D-056b: `sed -n 'a,bp' F | nl -ba -va` is a window on F.
- 6d979fd D-057: an `&&` chain whose links might not run is not split.
- 2d1106d D-053: grok 1.0.46 numbers every tenth line; a read is credited by its returned span when the shown text agrees.

Not built: D-056c (`cat A; echo; cat B` and newline sequences of different files; Antigravity's `false && cat F ; true` defeats the masked-link draft), 2b (repair turn), 2c (sight contract in the brief), D-054 (test rows in the production outbox).

## The data (from /Users/michaelboscia/.triumvirate/outbox.jsonl, REJECTED rows, 2026-09-01 to 2026-10-09)

- 271 REJECTED rows in total. 52 of them are test fixtures (tempdir paths under /private/var/folders). Those are test rows landing in the production outbox: a test isolation defect, and they inflate the count.
- About 167 real rejections, deduped by request_id.

| Bucket | Count | Whose fault |
|---|---|---|
| Partial read | 47 (grok 30, codex 15, gemini 2) | Mostly the GATE, see below |
| Touched the file, gate did not count it as a read | 31 | The GATE |
| Never opened, read other files only, or relative path | about 35 | Mixed. The relative-path ones are from the era before absolute paths were enforced |
| Zero tool calls at all | about 25 | The AGENT |
| Parser not on allowlist (codex, all on 09-02) | 11 | Already fixed |
| Modified files (grok 9, gemini 1) | 10 | The AGENT, or the sandbox was not in force |

The rejection rate among reviews cannot be computed from the outbox, because DONE rows mix reviews with plain asks.

### Gate false rejections seen in the receipts

1. **Ranged reads by tools other than the shell are never unioned.** `read_args_are_partial` (daemon/crates/triumvirate/src/agent_exec.rs:2401) treats ANY `limit` or `offset` on `read_file` as a slice. The union across windows (`codex_ranged_reads_cover_source`, :2110) only looks at reads made through the shell. Real receipts:
   - 139-line file, `{"limit":200}`, rejected as partial.
   - 1179-line file, `{"limit":1200}`, rejected as partial.
   - 240-line file, `limit 200` then `offset 200 limit 150`, rejected as partial.
   Grok splits its reads into chunks by default, so this bucket is mostly grok.
2. **Grok `cat FILE` through Bash is recorded with kind Bash, not ReadFile,** so `tool_call_read_source_in_full` filters it out. Receipt: `named by [Bash ok {"command":"cat /Users/.../codex-0154-exec-flags.diff"}], none counted`. The latest receipt is from 2026-10-08.
3. **Shell forms the codex parser does not recognize:** `sed -n '1,"'$p'"' FILE` (the quoting codex itself emits), `nl -ba A && nl -ba B`, a `for f in ...; do ... done` loop, and `wc -l X && sed -n '1,260p' X`. Some of these may be fixed already by D-017. Every receipt needs a replay before it is counted as live.

## Part 1. Capture what was done the first time

Today only 600 characters survive (agent_exec.rs:1117). The tool calls are not kept at all, so a rejection can never be replayed against a fixed gate.

**Proposal:** on every sight rejection, write `/Users/michaelboscia/.triumvirate/rejected/<request_id>.json` (mode 0600) containing:
- `response_text` (full)
- `tool_calls` (full, including args_json, kind and success)
- `parser_mode`, `required_sources`, `exec_cwd`, `agent`, `session_id`, `ts_ms`
- the gate's error

The error then names that path in place of the 600-character preview. The ledger stays text-free, as the jury decided (:338). This is a local file outside the ledger.

Two consequences:
- The human gets the full first answer, still labeled NOT A REVIEW.
- **A replay harness becomes possible:** run the current `enforce_reviewer_sight` over every stored rejection. That is the only honest way to count false rejections, and a regression test against the next gate change.

Both dispatch paths must get the change: the primary arm (:1068) and the second arm (:1561). One surface fixed and the other missed is the most common defect shape in this repo.

## Part 2. Fewer failed dispatches

In order of payoff:

### 2a. Fix the instrument first (gate false rejections)
- Union ranged `read_file` windows the same way as the codex shell windows. Interpret `offset`/`limit` per tool (0-based or 1-based must be checked against each CLI's docs, not guessed). A `limit` at or above the line count, with no offset, is a whole read.
- Classify grok's Bash `cat`/`sed` as a read before the ReadFile filter (or let `tool_call_read_source_in_full` accept shell-tool records whose command parses as a whole read, regardless of kind).
- Turn each receipt above into a test fixture. Negative controls stay: `limit:1` on a 500-line file, `tail -1`, and `offset` past the body must still fail.

### 2b. Repair instead of reject
When the gate fails on PARTIAL or NEVER-OPENED, and the backend can resume its session (codex, grok), send one follow-up turn into the SAME session:
"You did not read lines 241-568 of X (or: you never opened Y). Read them now, then restate your review, changing anything those lines change."
- Union the coverage of the original turn and the repair turn only. This is not the "reused worker" case the "only this turn counts" rule guards against: both turns belong to one dispatch and both sets of reads are verified records.
- One repair at most. If it fails, reject as today, and the stored file from Part 1 still holds both answers.
- Never repair MODIFIED or ZERO-TOOLS. A reviewer that wrote files, or looked at nothing, is not one to coax.

### 2c. Tell the reviewer the rule up front
Append a sight contract to every sighted dispatch: the absolute path of each required source, its line count, and the exact rule ("read whole: cat, or read_file with no limit; if you chunk, your windows must cover 1 to N with no gap"). The zero-tools and never-opened buckets are behavior, and the agent never sees the rule until after it has been rejected.

### 2d. Stop counting test rows
The 52 tempdir rows in the production outbox are a test isolation defect. Register it in daemon/docs/bugs/OPEN.md.

## Order of work
1. Part 1 (capture) must land before 2a, so 2a's effect can be measured by replay rather than estimated.
2. 2a, with the receipts as fixtures.
3. 2c (cheap; prompt only).
4. 2b (the biggest change; touches session resume on two backends).
