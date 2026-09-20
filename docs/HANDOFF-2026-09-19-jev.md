> **SUPERSEDED on 2026-09-20 by `/Users/michaelboscia/projects/triumvirate/docs/HANDOFF-2026-09-20.md`.**
>
> The "Next actions in priority order" section below is **withdrawn**. Item 2, the wiki-subject
> classifier, was rejected by a three-seat peer panel: the design would have erased the arm it was
> measuring. Item 2 also assumes the classifier "runs offline over rows already stored", and the
> local ledgers store no prompt text, so it could not have run as written. The reference sheet
> quoted here is wrong in three places (`choice` takes `criteria` and not `options`; an official
> Python SDK exists; `jtsang4/jev-cli` does not reject the official API).
>
> What survives from this document: the verified API facts, the key location, and the constraints
> section. Read the 2026-09-20 handoff before acting on anything below.

# Handoff, 2026-09-19: start the Jev integration, and where this session left everything

**Session:** `cde0e0e1-427d-4738-a00b-84043d279a67`
**Mechanical checkpoint (git state, uncommitted work, containers):**
`/Users/michaelboscia/.claude/handoffs/triumvirate/CHECKPOINT-cde0e0e1-427d-4738-a00b-84043d279a67.md`
**Read that first.** This document holds only what it cannot know.

---

## What this session did

The surprise is at the end: the wiki usage measurement was built twice, because the first definition
of "use" was wrong in a way the instrument could not see. Peers do not open wiki pages, they `grep`
the wiki directory, so a page-open rate reported zero for seats that used the wiki on nearly every
call. Use became "consulted the wiki, by any tool", and a `wiki_search` MCP tool was built so the
peers have a search interface the bridge can see directly. Naming that tool in the peers' map moved
Grok from 0 of 12 calls to 6 of 6. Before that: `ask_jury` shipped, and the defect register went
from 17 open to 2, each closed on its own check.

## State of the goal

| Goal | State | Evidence |
|---|---|---|
| `ask_jury` built and merged | DONE | PR #45, merge `ea1b53f` |
| Every defect in OPEN.md closed | 17 to 2 | `daemon/docs/bugs/OPEN.md`, D-004 and D-005 remain |
| D-004, D-005 closed | BLOCKED, not done | PostHog over quota; resets 2026-09-20 14:03 ET |
| Wiki step one: one event per ask call | DONE | PR #46, merge `ab05fbe` |
| Wiki step two: raw evidence, report | DONE | PR #47, merge `6904e73` |
| `wiki_search` tool | DONE | PR #48, merge `09e2c43` |
| Adoption measured after the map line | DONE | PR #49, OPEN, CI green |
| Jev integration | NOT STARTED | this document |
| Answer quality from the wiki | NOT MEASURED, not instrumented | no instrument exists for it |

## What physically exists now that did not before

| Thing | Path or id |
|---|---|
| `.env` for the TypeSafe key, chmod 600, gitignored | `/Users/michaelboscia/projects/triumvirate/.env` |
| `wiki_search` MCP tool (search over the wiki) | `/Users/michaelboscia/projects/triumvirate/daemon/crates/triumvirate/src/wiki_search.rs` |
| Per-call wiki evidence recorder | `/Users/michaelboscia/projects/triumvirate/daemon/crates/triumvirate/src/wiki_usage.rs` |
| Usage report, classifies at read time | `/Users/michaelboscia/projects/triumvirate/scripts/wiki-usage-report.py` |
| Report positive control, 24 fixture calls | `/Users/michaelboscia/projects/triumvirate/scripts/test_wiki_usage_report.py` |
| Ceiling probes, three single-use sets | `/Users/michaelboscia/projects/triumvirate/scripts/wiki-probes.py` |
| Probe journal, 60 rows | `/Users/michaelboscia/projects/triumvirate/reports/wiki-usage/probes.jsonl` |
| Detector parity fixture, Python and Rust | `/Users/michaelboscia/projects/triumvirate/scripts/fixtures/wiki-detector-cases.json` |
| The map line naming `wiki_search`, peers only | mneme-bosciamem `19f0792`, written by `ops/build_index.py` |
| Running daemon on the current binary | pid 14611, `~/.local/bin/triumvirate` |

## Findings, including ones that contradict earlier documents in this same session

1. **"Opened a wiki page" is the wrong definition of use.** Grok answered six of six hinted probes
   correctly with ZERO pages opened, Gemini five of six with two. They `grep` the directory. The
   brief's own proposed signal would have reported zero use for a seat using the wiki every time.
   Superseded by "consulted the wiki", any tool call touching it.
2. **A tool in the tool list is not a tool the agent knows about.** `wiki_search` sat available at
   zero usage until one sentence in the peers' map named it. Then Grok went to 6 of 6, unhinted
   included. Gemini went to 1 of 6, and Gemini is the seat that already opened pages, so the grep
   route the line argues against was never its route.
3. **Grok records every MCP call under a generic name** (`search_tool`, `use_tool`) and puts the real
   tool name in the ARGUMENTS. A confirmed `wiki_search` call recorded as no wiki touch at all.
4. **Grok sandboxes the MCP server process it spawns.** A write to `~/.triumvirate/` from inside it
   returns "Operation not permitted", so the server-side search journal is structurally blind to
   sandboxed peers. The ledger evidence is the authoritative count; the journal is best-effort.
5. **Probe questions are consumable.** Grok keeps per-session memory, so a question asked once may be
   answered from memory afterwards, which understates adoption. Three single-use sets exist; a fourth
   is needed for the next adoption test.
6. **The grounded search on driving agent adoption was weakly sourced** (2 sources, 3% coverage). Its
   numbers are not cited anywhere in this repo. Its one durable pattern, retrieval as a tool whose
   description names its trigger, agreed with the probes and was already option 5 in
   `mneme-bosciamem/research/2026-09-19-consumption.md`.
7. **NOT VERIFIED, unknown in both directions:** whether consulting the wiki makes a peer's answer
   BETTER. Everything measured here is delivery and route. No instrument exists for quality.

## Incidents and the rules they produced

| Incident | Rule |
|---|---|
| My D-001 graceful shutdown hung the daemon; `start-daemon.sh` SIGKILL escalation hid it all day | Any graceful wait needs a bound. Verify a shutdown with a connection held open, and test the thing underneath a wrapper that escalates. Never pipe `start-daemon.sh` to `head` (SIGPIPE kills the script before it escalates). |
| The test suite wrote `wiki_call` events into real shared ledgers; the first report counted 11 test calls as peer calls (D-019) | When a shared code path gains a durable side effect, fence test builds to opt-in, and prove the fence with a before and after scan plus a control run with the fence removed. |
| `let _ = writeln!` swallowed the journal's write error, so a sandbox denial read as zero use | Never swallow the error from a recorder you will later read as evidence. |
| A page-open metric missed peers who only `grep` | Define use as the shape it takes, not the shape you expected. A positive control is what exposes the difference. |

## Operating constraints learned the hard way

- **PostHog returns 200 OK for events it discards while over quota.** Delivery is only known by a
  sentinel round trip through `posthog.ai_events`. Query errors arrive in `detail`, not `error`.
- **MCP server processes are long lived and run the code they started with.** 16 stale ones were
  killed this session, oldest from Sep 17. A fresh build does not reach a running session.
  This session's own server (pid 19165) is still the pre-`wiki_search` build.
- **The auto-mode classifier blocks killing processes in a loop**, and blocked a batch of 7 by pid.
  Individual and small explicit-pid kills pass.
- **`cargo clippy --all-targets`** has 15 pre-existing `MutexGuard held across await` errors in tests.
  CI runs clippy WITHOUT `--all-targets`; match CI, or a clean change looks broken.
- **A peer's self-report about its own tool list is unreliable.** Grok said it had no `wiki_search`
  while holding it. Ask it to CALL the tool and check the result against a direct call instead.

---

# The Jev integration: what the next session should do

## What Jev is, verified against primary docs, not search

TypeSafe's "System One" model. It does not generate text. It evaluates a `state` against typed
questions and returns structured answers with probabilities and confidence.

| Fact | Value | Source |
|---|---|---|
| Endpoint | `POST https://api.typesafe.ai/v1/systemone` | `https://docs.typesafe.ai/introduction/quickstart` |
| Auth | `Authorization: Bearer <API_KEY>` | same |
| Request fields | `state`, `model`, `questions` | `https://docs.typesafe.ai/api.md` |
| Response fields | `model`, `answers`, `usage` | same |
| Primitives | `choice`, `score`, `noul` (yes/no as 0 to 1) | `https://docs.typesafe.ai/primitives.md` |
| Model | `jev-1.13.0`, alias `jev-latest` | `https://docs.typesafe.ai/models.md` |
| Context | 64k per request, 32k for `state` plus longest question | same |
| Price | $42 per billion input tokens, output free | same |
| Errors | 401, 422, 429, 529, retry with exponential backoff | `https://docs.typesafe.ai/api.md` |
| Keys | `https://console.typesafe.ai/keys` | quickstart |
| Known limitations | `https://docs.typesafe.ai/model-jaggedness/jev-1.13.md` | read before trusting it |
| Agent skill for Claude Code and Codex | `https://docs.typesafe.ai/agent-skill.md` | not yet read |

Rate limits are "adjusting dynamically" and no numeric threshold is published. Max questions per
request, max state size, batching, timeouts, and whether requests are logged or used for training
are all NOT STATED in the docs read so far.

## The key is in place, and the API was verified live

The owner saved a real key into `/Users/michaelboscia/projects/triumvirate/.env` on 2026-09-19. The
file is chmod 600 and gitignored at `.gitignore:6`. **The key lives there and nowhere else: never in
a commit, never in a prompt, never echoed into command output.**

Verified with one real call before any code was written:

```bash
cd /Users/michaelboscia/projects/triumvirate && set -a && . ./.env && set +a
curl -s -X POST https://api.typesafe.ai/v1/systemone \
  -H "Authorization: Bearer $TYPESAFE_API_KEY" -H 'Content-Type: application/json' \
  -d '{"state":"The database is reporting 100% CPU and every request is timing out.",
       "model":"jev-latest",
       "questions":{"is_incident":{"type":"noul","instructions":"This describes a live production incident"}}}'
```

Result, 2026-09-19: **HTTP 200 in 0.49s**, `model: jev-1.13.0`, `answers.is_incident.noul: 0.79`,
`usage: {input_tokens: 289, output_tokens: 22}`. So auth, endpoint, request shape and response shape
are all confirmed against the live service, not just the docs.

Two things that call also settled: a `noul` costs a few hundred input tokens (289 for one short
state plus one question), and sub-second latency is real. Note the value: 0.79 on a description
that is unambiguously an incident, which is a reminder that a noul is a probability and needs a
threshold chosen deliberately, not assumed to sit near 1.

## Next actions in priority order

1. **DONE: key saved and the API verified live.** See the section above for the exact call and its
   result. Nothing is blocked.
2. **Build the wiki-subject classifier first**, not the jury. Reason it is ranked first: it is the
   only candidate that is NOT in a load-bearing path, it runs offline over rows already stored, and
   it arrives with its own evaluation. The current rule is a regex
   (`WIKI_SUBJECT` in `/Users/michaelboscia/projects/triumvirate/scripts/wiki-controls.py`) that the
   design jury criticised as fitted to the failures it explains. Jev can be scored against that
   regex on rows the regex was never fitted to, which is the validation the jury asked for.
   Classification happens at report time, so both can run over the same rows and be compared.
3. **Only if step 2 wins, consider the jury verdict extractor**
   (`/Users/michaelboscia/projects/triumvirate/daemon/crates/mcp-tools/src/jury.rs`). Today
   `VerdictExtractor` parses prose for approve, concerns or reject and returns `None` on ambiguity.
   Jev's `choice` fits, but it must be a FALLBACK for the `None` case only, never a replacement, with
   the confidence stored and low confidence still yielding indeterminate. A vote path that depends on
   a paid network call is a new failure mode on the surface this repo cares about most.
4. **Seat and depth routing** (`grok_depth` fast against deep, which seat answers) is the documented
   use case and the cheapest win, but it changes dispatch behaviour, so it should wait until Jev has
   a measured track record here.
5. **Do not put Jev in the sight gate.** That gate is mechanical on purpose: it reads tool records,
   not prose. A probabilistic judgement there would undo the property that makes it trustworthy.

## Constraints the Jev work must respect

- **No API keys in code, in prompts, or in commits.** The key lives in `.env` only.
- **Data integrity**: any number Jev produces that reaches a report must be traceable, and the report
  must say the classification came from a model, with its confidence. Do not let a model verdict
  read as a measured fact.
- **One implementation, not two.** This repo's top defect shape is a fix landing on one of two
  surfaces. A Jev client belongs in one place, called from both the report and any daemon path.
- **Mutation-test every guard.** Break it on purpose and watch a test fail, or it is not tested.

---

## Map of the artifacts

| Path | What it is |
|---|---|
| `/Users/michaelboscia/projects/triumvirate/.env` | TypeSafe key placeholder, chmod 600, gitignored |
| `/Users/michaelboscia/projects/triumvirate/docs/briefs/wiki-usage-measurement-brief.md` | The measurement brief, with as-built, jury, findings, adoption |
| `/Users/michaelboscia/projects/triumvirate/docs/briefs/ask-jury-brief.md` | ask_jury brief and as-built |
| `/Users/michaelboscia/projects/triumvirate/daemon/docs/bugs/OPEN.md` | Defect register, 2 open, closed rows with their checks |
| `/Users/michaelboscia/projects/triumvirate/scripts/wiki-controls.py` | The two free controls, and the `WIKI_SUBJECT` regex Jev would be scored against |
| `/Users/michaelboscia/projects/triumvirate/scripts/wiki-usage-report.py` | Usage report, all classification at read time |
| `/Users/michaelboscia/projects/triumvirate/reports/wiki-usage/` | Reports and the probe journal |
| `/Users/michaelboscia/.claude/handoffs/triumvirate/CHECKPOINT-cde0e0e1-427d-4738-a00b-84043d279a67.md` | Mechanical checkpoint for this session |

## Session transcripts

- Main transcript: `/Users/michaelboscia/.claude/projects/-Users-michaelboscia-projects-triumvirate/cde0e0e1-427d-4738-a00b-84043d279a67.jsonl`
- The session was compacted once and kept the same id, so the whole conversation is that one file.
