# Brief: measure whether the wiki map reaches a peer, and whether the peer used it

Owner's request, 2026-09-19: the Claude side of this measurement shipped today, and the same thing
is wanted for the peers that Triumvirate calls. This brief is the peer half. It is the companion to
`/Users/michaelboscia/projects/triumvirate/docs/briefs/wiki-injection-brief.md`, which delivers the
map; this one says how to tell whether delivering it did anything.

Read first, because both carry numbers and lessons this brief depends on:
- `/Users/michaelboscia/projects/mneme-bosciamem/research/2026-09-19-usage-measurement.md`, section "Status: built"
- `/Users/michaelboscia/projects/mneme-bosciamem/ops/usage_events.py`, the Claude-side scanner
- `/Users/michaelboscia/projects/localmemory/docs/MEASURE-LOOKUP-EFFECT-2026-09-19.md`, the Graphiti half of the same pair

## Where things stand

| seat | how the map reaches it | delivery recorded | use measured |
|---|---|---|---|
| Claude | `@` import in `/Users/michaelboscia/.claude/CLAUDE.md` | yes, `state/deploy.json` in the mneme repo | yes, nightly scan of both transcript trees |
| Codex | managed block in `/Users/michaelboscia/.codex/AGENTS.md` | no | no |
| Gemini, via the Antigravity CLI | managed block in `/Users/michaelboscia/.gemini/GEMINI.md` | no | no |
| Grok | `/Users/michaelboscia/.grok/rules/bosciamem-wiki.md`, a global user rule, since 2026-09-19 15:47 UTC | yes, `state/deploy.json` in the mneme repo | no |

All four Primes now receive the map from files. Three of them are measured for neither use nor effect,
and none of the three leaves a transcript on this machine. The bridge is the only place that can see either side of those
exchanges, which is why this work belongs here and not in the mneme repo.

## Verified seams

Read out of the tree on 2026-09-19, not from memory:

- `daemon/crates/triumvirate/src/agent_exec.rs:1553`, `inject_tool_marker_prompt(user_prompt) -> String`, called at line 488 as `let execution_prompt = inject_tool_marker_prompt(&req.message);`. This is where delivery happens and where the delivery half of the event is known.
- `daemon/crates/triumvirate/src/agent_exec.rs`, around line 1418, the `AskAgentResponse` construction. It already carries everything the use half of the event needs: `response`, `agent`, `answered_by_agent`, `answered_by_backend`, `degraded_from_backend`, `degradation_reason`, `request_id`.
- `daemon/crates/ledger/src/lib.rs:54`, `LedgerStore::ingest_event(RawEvent)`. `RawEvent` is `{session_id, event_type, sequence, timestamp, payload_json}`, defined at `daemon/crates/shared-types/src/ledger.rs:5`.
- `daemon/crates/ledger/src/store.rs:166`, the database path: `<project_root>/.triumvirate/ledger.db`.

## The finding that changes the order of work: the ask path has never written an event

Corrected 2026-09-19, same day, after a first version of this brief overstated it. The first pass
said the ledger's event sink had no producer at all, on the evidence that the `events` table is
empty in all 13 ledger databases on this machine. That evidence was too weak for the claim. Here is
what is actually true, from the tree:

- `ingest_event` **does** have production callers, in `daemon/crates/fleet/src/orchestrator.rs` and `daemon/crates/fleet/src/recovery.rs`. The sink works and is exercised.
- What it does not have is a caller on the **ask path**. `agent_exec.rs` uses the ledger for records, summaries and reviews, and never writes an event. So the exact path this brief needs is the one path that has never written to the sink.
- The table reads empty today for two ordinary reasons, not a defect: fleet runs are rare, and `gc.rs` deletes events older than `EVENT_RETENTION_DAYS`, which is 30. The repo's own ledger shows an events autoincrement high-water mark of 15 against 0 current rows, which is exactly what retention looks like after the fact.

The practical instruction is unchanged and the reason for it is now sharper. **Step one is still to
prove the sink from the ask path**: write one event during a real `ask_agent` call, then read it back
out of the database the daemon actually opened, and print which project root that was. Do not infer
that it works because the fleet crate's calls compile.

Two consequences worth carrying into the design:

- **Retention is 30 days.** A measurement that wants a before-and-after across a longer window cannot live only in `events`. Either summarise into a table gc does not sweep, or write the report artifact to disk on each run, which is what the Claude side does.
- **An empty table is not evidence of a missing producer.** This brief made that mistake in its first version. Check `sqlite_sequence` for whether a table ever held a row, and check the retention rule, before concluding anything from a zero.

## The signal, and why it is weaker here than on the Claude side

On the Claude side the signal is unforgeable. The wiki is an index, so opening a page IS the
citation, and a tool call that opens `<repo>/<page-id>.md` cannot happen by accident.

The bridge cannot see that. It sees the prompt it sent and the text that came back. Codex and Gemini
can open files, but their own file reads never reach Triumvirate. Grok cannot open files at all. So
the only available peer signal is **page ids appearing in the response text**, which is much weaker
and needs three guards.

1. **Word boundaries, or paths will fire it.** A page id inside `research/quiet-failures-and-absence-monitoring-notes.md` is not a citation. The Claude-side scanner uses `(?<![\w/-])(id1|id2|...)(?![\w-])` and has a test fixture whose whole job is that near miss.
2. **Map recitation is the dominant false positive, and the Claude side never faced it.** The injected map names all 20 page ids. An agent that quotes the map back scores 20 citations for zero use. Rule: if a response names five or more distinct page ids, class it as recitation, record it with that label, and exclude it from the use numerator. Record the count either way so the rule can be checked later against real responses rather than argued about.
3. **A floor is required before any rate is believed.** See the next section.

## Two free controls that should be used before writing any Rust

**Grok's history before 2026-09-19 15:47 UTC is a true negative control.** Until then it received no map
from any route. Run the detector over Grok responses from before that time: it must return zero. If it
fires there, the detector is wrong. Live Grok calls are no longer a negative control, because Grok now
loads the map from its global rules folder.

**The instruction files have a known start date.** Codex and Gemini have carried the map since
2026-09-19 14:23 UTC. Calls before that are the coincidence floor for those two seats, in the same
way the 153 pre-import Claude sessions were, which opened a page zero times across 33,251 turns.

## Credit the seat that answered, never the seat that was asked

On 2026-09-19 the bridge's Gemini backend hit quota, the breaker opened, and Gemini calls were
silently answered by Codex. It was caught only because `answered_by_agent` recorded it. A usage
measurement that keys on the requested agent would have credited Gemini with Codex's behaviour and
produced a per-seat rate that was quietly fiction.

So: key every row on `answered_by_agent`. When it differs from `agent`, mark the row degraded and
leave it out of per-seat rates entirely rather than assigning it to either seat. Carry
`answered_by_backend` and `degraded_from_backend` in the payload so that decision is auditable
afterwards.

## Population hygiene, both lessons learned the hard way on the Claude side

**Enumerate the denominator with a command, never from memory.** The Claude-side scan first computed
its floor from one config directory and got 62 sessions. There are two, because
`/Users/michaelboscia/.claude-masterffl/CLAUDE.md` is a symlink to the global one, and the real
figure is 153. The scan exited 0 and printed a plausible number the whole time. That is now an HNT
entry, and the identical trap is already sitting here: the ledger is per project root and there are
13 of them. Any reader that opens one database is wrong by construction. Glob them all, and report
how many were found alongside the totals so a future drop is visible.

**Label the call classes instead of merging them.** On the Mac, 106 of 262 sessions turned out to be
SDK harness jobs with no person in them, which would have diluted every rate by about forty percent.
The bridge's version of that distinction is the reason a call was made: an interactive `ask_agent`,
a `dispatch_codex` worktree job, a mandatory peer review, a fleet task. Those are different
populations and a single blended rate across them means nothing. Put the class in the payload.

## The event

One row per call, emitted at the response seam so delivery and use land together and nothing has to
be joined later. Suggested `event_type` is `wiki_call`.

Payload, ids and counts only, no prose and no page content:

- `request_id`, `session_id`
- `agent_requested`, `answered_by_agent`, `answered_by_backend`, `degraded` (bool), `degradation_reason`
- `call_class`: ask, dispatch, peer_review, fleet
- `injected` (bool), `inject_mode`: none, map
- `map_generated` (the date on the map's first line), `map_pages`, `map_bytes`
- `skip_reason` when nothing was injected: stale, missing, disabled, per_agent_none
- `page_ids_referenced` (array), `page_ids_distinct` (int), `recitation` (bool)
- `latency_ms`, `response_chars`

`map_generated` matters on its own. A stale map that keeps being injected is the fallback that
outlives its reason, and the injection brief already calls for refusing to inject past
`wiki.max_age_days`. Recording the date is what proves the refusal actually fires.

## The report

`scripts/wiki-usage-report.py`, reading every ledger found by globbing, writing
`reports/wiki-usage/<date>.md` in this repo. Mirror the Claude-side report's columns so the two can
be read side by side: arm, calls, calls that referenced a page, page references, recitations,
top pages. Arms here are `delivered` and `baseline` split on the per-seat injection start date, with
`degraded` held apart.

## A positive control is mandatory, not optional

A scanner that reports zero is indistinguishable from a broken one. The Claude-side scanner ships
with `ops/test_usage_events.py`: a fixture containing one of every signal and one of every near
miss, which fails if any count comes back zero. Two of its ten checks were wrong on the first run,
and finding that before shipping is the entire point.

Build the same thing here before trusting any number. The fixture needs, at minimum: a clean
citation, a path-embedded near miss, a recited map, a degraded row, and a call from a second ledger
database.

## Acceptance

- One event per call reaches the ledger the daemon actually opened, proven by reading it back, with the database path named in the run output.
- The reader finds all 13 ledgers and says how many it found, and does not treat an empty events table as evidence of anything without checking sqlite_sequence and the 30 day retention rule.
- The detector returns zero on Grok responses from before 2026-09-19 15:47 UTC.
- A response that recites the map is labelled `recitation` and does not appear in the use numerator.
- A degraded call is keyed to the seat that answered and is excluded from both seats' rates.
- A map older than `wiki.max_age_days` is not injected, and the event records `skip_reason: stale`.
- The positive-control fixture passes with no zero counts.
- No page text, no prompt text and no response text is written to the ledger.

## Not in scope

Scoring whether the peer's answer was better for having the map. That is the paired regeneration,
and the Graphiti side has already shown it is the method that works and the ablation is the one that
does not. It needs a use signal first, which is what this brief produces.

## Findings from the two free controls (2026-09-19), run before any Rust

Run by `scripts/wiki-controls.py` over successful `$ai_generation` rows in
`posthog.ai_events.output_choices`. Report: `reports/wiki-usage/controls-2026-09-19.md`. The
detector imports its page list from `mneme-bosciamem/ops/usage_events.py:page_ids` and uses that
file's bare-id pattern character for character, and it passed a seven-case positive control
(two citations, three near misses, a recited map, a no-page response) before any zero was read.

**The negative control FAILED on its first run, and that was the most useful result of the day.**
14 Grok responses from before 15:47 UTC named page ids, 5 of them recitations. Every page id cited
was absent from its prompt, and several were produced on 2026-09-06 and 2026-09-08, before the
strings were even committed to the mneme repo (first commit 2026-09-17). The cause is a population
this brief did not model:

**Calls whose SUBJECT is the wiki.** The mneme blind-labeler jury ("You are a blind labeler. Read
.../mneme-bosciamem/gold/LABELING-BRIEF.md, then BATCH-2.md") returns JSON whose `label` field IS a
page id. The wiki bootstrap review had Grok reading the taxonomy files. In both, naming page ids is
the job. The ids came from files the peer opened, so the prompt string held only a path and an
echo check against the prompt found nothing.

| seat | pre-map calls | wiki-is-the-subject | hits | hits that were wiki-subject |
|---|---|---|---|---|
| grok | 434 | 230 (53%) | 14 | 14 |
| gemini | 458 | 188 (41%) | 7 | 7 |
| codex | 270 | 2 | 0 | 0 |

With that population excluded, **the negative control is zero** (Grok, 204 ordinary calls) and
**the coincidence floor is zero** for every seat (Codex 268, Gemini 270 ordinary calls). The
detector was right; the control failed on an unmodeled population.

Consequences for the design, before a line of Rust:

1. **Wiki-subject calls must be excluded from every rate** (CORRECTED below: not as a field
   stored at emit time). Without the exclusion the measurement is roughly half contaminated for
   Grok and 40% for Gemini, and the contamination is all in one direction: it manufactures use.
   Today's rule is a match on the USER TASK: the prompt names the wiki's own repo or build
   artifacts (`WIKI_SUBJECT` in `scripts/wiki-controls.py`).
   **RETRACTED:** this item first said the rule could be made "exact" at the response seam as
   "wiki-subject if its required sources or its OPENED FILES are under the wiki's repo". That is
   circular and would have zeroed the metric by construction: opening a page IS the use signal,
   so every genuine lookup would classify as wiki-subject and be excluded. Gemini and Grok each
   found it independently. Subject must be read from the task, never from opened pages.
2. **"Grok cannot open files at all" is wrong.** It read the labeling brief, the batch files and
   the taxonomy files here, and made 80 file-reading tool calls in one review the same day.
3. **"Their own file reads never reach Triumvirate" is wrong, and this is the good news.** The
   sight gate works precisely because they do: `ToolCallRecord` carries each read's arguments,
   and a receipt the same day read `bound to [lines 1-260 of .../ask-jury-brief.md]`. So the
   UNFORGEABLE signal this brief assumed only the Claude side had, a tool call that opens
   `<wiki repo>/<page-id>.md`, is available for the peers at the response seam, wherever the
   parser records tool calls (`PARSER_MODES_WITH_TOOL_RECORDS`). Proposal: `pages_opened` as the
   primary use signal, with the text match demoted to a secondary one. That also removes most of
   the recitation problem, since reciting the map does not open a page.
4. **Echo must be judged against what the peer read, not only against the prompt string.** A page
   id that arrives in an opened file and leaves in the response is echo, and the prompt-only check
   cannot see it.
5. **A zero floor makes the signal strong.** Ordinary pre-map calls named a page id zero times in
   742 calls across three seats, so after the exclusions above any reference in a delivered call
   is unlikely to be coincidence.

**Not yet measurable:** the DELIVERED arm. Almost nothing reached PostHog after the map start
times, because the account has been over quota since 2026-09-16 (D-018). The controls are
unaffected because every row they use predates the outage.

## Jury on the design, 2026-09-19: all three answering peers said "neither A nor B"

Asked through `ask_jury` (codex, grok, gemini, deepseek), with a self-contained brief laying out
Position A (PAGES_OPENED primary, `call_subject` frozen at emit time) and Position B (TEXT
primary, classify at report time), and asking what BOTH missed. Outcome `majority`, 3 of 4 seats,
all three voting **C**. Codex was reported `unavailable` rather than substituted: its weekly usage
limit resets 2026-09-20 14:03.

**Where they agreed.** Store the raw evidence at emit time and classify at report time (B was
right about that, and it is the one decision that cannot be undone later). Do not publish a rate
yet. Promote neither signal on today's evidence.

**What they found that I had missed, each checked where it could be checked:**
- **The circular rule above.** Retracted.
- **The subject regex matched the map's own first line.** `bosciamem wiki` is in both. The rule
  reads only the user's prompt today, and peers get the map from instruction files, so no
  historical result changed; but the moment the map is delivered through the prompt, every
  delivered call would classify as wiki-subject and the delivered arm would vanish. The term is
  removed, `scripts/wiki-controls.py` now refuses to run if any rule term matches the map, and
  restoring the term makes it refuse. Re-run after the change: identical classifications.
- **TEXT was validated for a regime that no longer exists** (DeepSeek, Grok). The controls prove
  the regex does not fire when the map is ABSENT. Once all 20 ids sit in context permanently,
  naming one or two of them is map echo, and the five-id recitation rule does not catch that.
  So "TEXT is the only validated instrument" overstated it: it is validated for specificity
  before delivery, not for anything after.
- **No positive control, no ceiling** (DeepSeek). Nothing shows either signal FIRES when a peer
  definitely uses a page. Without that, a near-zero rate cannot tell "peers ignore the wiki" from
  "the instrument is dead". Proposed: a placebo map (no useful content) and a task answerable
  only from a page's body.
- **An empty tool-call list is unobservable, not zero** (Grok). A parser that records no tool
  calls reports zero opens for every call it serves, which reads as "use dropped to zero". The
  event must store `parser_mode` and whether tool records were present.
- **Prompt-mandated opens are a third class** (Grok). "Read this page" is obedience, not
  map-driven lookup, and neither position modelled it.
- **PAGES_OPENED does have baselines** (all three, differently): the page files did not exist
  before 2026-09-17, so opens were zero by construction; a call with `tv_tool_calls = 0` has zero
  opens exactly; and going forward, a holdout that withholds the map from a share of calls gives a
  contemporaneous floor. DeepSeek adds a per-peer canary id, which no prior knowledge, recitation
  or file echo can produce.

**Where they disagreed.** Whether the 230-versus-14 scale rescues the fitted rule: Gemini says
completely; Grok says it rescues it from tautology but not enough to freeze it (Codex, at n=2, is
not a holdout); DeepSeek says it proves precision and says nothing about recall. And on the
eventual primary signal: Gemini favours PAGES_OPENED, Grok would make it the use definition only
where tool records exist, DeepSeek would promote neither until a placebo and a canary have run.


## As built, step two (2026-09-19)

Every answered ask call's `wiki_call` event (schema 2) carries an `evidence` object. Ids and
flags only, no text, and no judgement: all classification is in the report.

- `parser_mode`, `tool_records`, `reads_classified`: whether this call's parser could see a read
  at all (Grok, jury). `pages_opened` is `null`, not `[]`, when it could not.
- `backend`: named on the direct path too. Step one's live run had `None` for a direct agy call.
- `pages_opened`: pages read, judged by the sight gate's own matcher (`tool_call_touched_source`),
  so every adapter goes through the one implementation that already decides "was this source
  read". Not a second parser per adapter.
- `text_ids`, `prompt_ids`: page ids in the response and in the prompt, from
  `daemon/crates/triumvirate/src/wiki_usage.rs`. The Python and Rust detectors are both checked
  against `scripts/fixtures/wiki-detector-cases.json` (13 cases); drifting either one fails.
- `prompt_paths`: absolute paths in the prompt, so the wiki-subject rule runs at report time
  without storing the prompt.
- `wiki`: dir, map date, page count, map bytes; or `{"error": ...}` when the page list cannot
  load, which the report holds apart rather than scoring as zero use.

`scripts/wiki-usage-report.py` finds every ledger (21 today, listed with rows ever written from
sqlite_sequence so pruning is not read as silence), keys on the seat that answered, and holds
apart: failed, schema 1, missing evidence, unloadable wiki, test harness, degraded, prompt named
a page, wiki is the subject, recitation. It prints counts per seat, arm and class, and **publishes
no rate**. Positive control: `scripts/test_wiki_usage_report.py`, 16 fixture calls in two
ledgers, fails on any wrong or zero count. Every guard was mutation-tested.

**Live proof:** a direct agy call recorded `backend: agy`; a Grok call told to read
`verify-by-reading-back.md` recorded it in `pages_opened` and `text_ids` through the real
`grok-streaming-json` parser. A repeat on the reinstalled binary (`aws-operations`) matched and
carried no harness mark.

**Found on the way:** D-019, the test suite writing events into real ledgers. Closed; see
`daemon/docs/bugs/OPEN.md`.

**Not built, and why.** The jury's placebo map, canary id and holdout all need control over
WHAT MAP a peer receives, per call. Peers get the map from their own instruction files, not
through the bridge, so the bridge cannot withhold or swap it for one call. That needs the
injection mechanism, which is a separate brief. Until those exist no rate is believable, and
the report says so at the top. Also still open: the brief's `skip_reason: stale` check belongs to
that injection mechanism for the same reason.

## The rate, and the finding that changed its definition (2026-09-19)

Owner's decision: the map stays in the peers' own instruction files, so there is no holdout and no
placebo. The floor comes from construction instead: page files did not exist before 2026-09-17, a
peer without the map has no way to know a page path, and prompt-named pages are excluded. The
positive control is a set of ceiling probes, `scripts/wiki-probes.py`: six questions whose answers
live in one page body and in no instruction file (checked before a call is spent, and the check
refuses a probe whose answer is reachable without a lookup). Each runs `hinted` ("check the
knowledge base listed in your instructions") and `unhinted`.

**The first probe run killed the original definition.** "Opened a wiki page" was the proposed use
signal. Grok answered SIX of six hinted probes correctly with zero pages opened, and Gemini five of
six with two. They were not reading pages: they were running `grep` and shell commands against the
wiki directory and answering from the search output. A page-open rate would have reported zero use
for seats that used the wiki on nearly every call. That is the same shape as the D-018 lesson: the
instrument measured something adjacent to the question.

So the event now records `wiki_tool_calls` (schema 3): every successful tool call whose arguments
name the wiki, with its kind, the page files its arguments name, and whether it touched the index.
File names only. **Use is now "consulted the wiki", any tool call touching it; opening a page is one
kind of consultation and is still reported separately.** A seat's rate is published only when a
HINTED probe shows the instrument SEEING that seat consult the wiki. Requiring a page open there
would have failed Grok for using the wiki the way Grok uses it.

Second run, on the instrumented binary (12 probes per seat per variant, two runs):

| seat | variant | consult observable | consulted | opened target | answer matched |
|---|---|---:|---:|---:|---:|
| gemini | hinted | 8 | 6 | 2 | 10/12 |
| gemini | unhinted | 6 | 2 | 0 | 7/12 |
| grok | hinted | 6 | 5 | 0 | 12/12 |
| grok | unhinted | 6 | 1 | 0 | 8/12 |

Both instruments are proven. Organic rates are still 0/0: the only organic schema-2 calls predate
`wiki_tool_calls`, and a legacy row counts as unobservable rather than as zero use. The rate accrues
from here. Codex probes wait on its quota reset (2026-09-20 14:03 ET).

**What the probes say beyond the instrument.** Hinted beats unhinted on every seat, which is the
known shape: peers consult when told to, and mostly do not otherwise. Grounded search on how others
drive consultation (run 2026-09-19, LOW grounding: 2 sources, 3% coverage, so its numbers are not
cited here) offers one durable pattern that matches this evidence: retrieval gets used when it is a
TOOL whose description says when to call it, not a file tree the agent may search. That is already
option 5 in `mneme-bosciamem/research/2026-09-19-consumption.md` (a `wiki_search` tool, the Letta
archival pattern), it is the one option never built, and the probes show peers reaching for exactly
that shape by hand with `grep`.

## `wiki_search`, and what building it measured (2026-09-19)

The probes said peers reach for a search interface by hand, so they got one: `wiki_search`, an MCP
tool on the existing bridge (`daemon/crates/triumvirate/src/wiki_search.rs`). Not a new server:
codex, gemini and grok already have the Triumvirate MCP server registered, so the tool reaches all
three with no config change. Term matching over 20 markdown files, no index and no embedding
service to go stale. The description names its trigger ("CALL THIS FIRST, before answering
anything about how things are done here... do NOT grep or ls the wiki directory"), because a
description that only says what a tool IS does not get called.

**The first live call was ranked wrong, and it showed the bug.** "langfuse port homebox" returned a
1050-line page of the word "supports" above the page holding the answer: substring matching, so
`port` matched `supports`, and volume beat relevance. Now terms match on word boundaries (the same
rule as the page-id detector) and pages rank by how many distinct query terms they cover before
any volume count. Both are pinned by tests built from that failure.

**Three measurement defects, all found live, all silent:**
1. A confirmed peer `wiki_search` call recorded as NO wiki touch. Grok records every MCP call
   under a generic name (`search_tool`, `use_tool`) and puts the real tool name in the ARGUMENTS.
   The filter read the name field only. Both places are now checked.
2. Nothing on this side could say why. The event now carries `tools`, every tool name in the turn
   with a count, names only. A measurement that cannot be debugged from its own output is the
   failure this brief exists to prevent.
3. The server-side journal silently wrote nothing for peer calls, because the write error was
   swallowed. Made loud, and the log then named the cause in one line: Grok SANDBOXES the MCP
   server it spawns, so a write to `~/.triumvirate/` returns "Operation not permitted". The
   journal is therefore best-effort and structurally blind to sandboxed peers; the ledger evidence
   is the authoritative count.

**Adoption so far: zero unprompted.** Twelve fresh probes (facts never asked before, because Grok
keeps per-session memory and the first probe set is burned for adoption testing) across grok and
gemini: every answer correct, `wiki_search` called zero times. Both seats still grep. Told to call
it explicitly, both do, and the answers are real (Grok's returned line numbers matched the tool's
output exactly). So the tool works and is reachable; nothing yet TELLS the peers it exists, because
the map in their instruction files describes a directory of files, not a search tool.

**The next lever, not taken here:** one line in the map that names `wiki_search` as the way to look
something up. That is a change to `ops/build_index.py` in the mneme repo, which owns the map, and
is the owner's call. The instrument is now in place to measure whether it works: `via wiki_search`
is a column in the report.

## The map line, and the adoption it bought (2026-09-19)

One sentence added to the peers' map block (and ONLY the peers': `_index.md`, which Claude imports,
is untouched, because the Claude-side scanner measures use by page OPENS and pointing Claude at a
search tool would blind its own instrument):

> To look something up here, call the `wiki_search` tool (Triumvirate MCP) with the words you expect
> on the page... Prefer it over grepping or listing this directory, and call it before answering
> from memory or saying you do not know.

Written by `ops/build_index.py` in the mneme repo, so the nightly keeps it in place. Measured on a
THIRD probe set, three facts never asked before (each set is single-use for adoption: a peer that
already knows the answer has no reason to look anything up):

| seat | before, 12 fresh probes | after, 6 calls | answers correct |
|---|---:|---:|---:|
| grok | 0 used `wiki_search` | **6 of 6** | 6 of 6 |
| gemini | 0 used `wiki_search` | **1 of 6** | 4 of 6 |

Grok switched completely, unhinted as well as hinted. Gemini mostly did not, and it is the seat that
already opened pages, so its route to the wiki was never the one the line argued against. Both
numbers come from the bridge's own record of the peers' tool calls, not from what the peers said.

Worth stating plainly: nothing here yet shows the wiki made an answer BETTER. It shows delivery
(the map reaches them), route (which mechanism they use), and that naming a tool in the instructions
moved one seat from zero to always. Quality is the next question and it is not instrumented.
