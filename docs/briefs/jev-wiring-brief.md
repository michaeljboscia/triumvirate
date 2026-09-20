# Wiring Jev into Triumvirate

**Written:** 2026-09-19. Supersedes the "Next actions" section of
`/Users/michaelboscia/projects/triumvirate/docs/HANDOFF-2026-09-19-jev.md` on two points, both
because of facts checked against the code and the live API rather than against the handoff.

---

## STATUS: Phase A is WITHDRAWN. Peer panel 2026-09-19.

A three-seat panel (Antigravity, Grok deep with sight gate, DeepSeek) reviewed this document and
the client survey. **Two seats independently rejected the Phase A plan and one found a defect in
it that is specific to this repo and verified below.** Do not build Phase A as written.

What survives: the client decisions (direct HTTP, write the Rust client, pin `jev-1.13.0`, retry
429 and 529 only, 422 fails loudly, Jev never in the sight gate). What does not survive: the
wiki-subject classifier as the first integration, the glossary-in-`state` design, and the
disagreement-only labelling scheme. Details in "Peer panel findings" below.

## Open items, first

| # | Open item | Why it is open |
|---|---|---|
| O-0 | **The glossary re-creates the contamination `WIKI_SUBJECT` was built to prevent** | VERIFIED. Grok. See panel finding 1. Blocks the whole Phase A design, not just a parameter of it. |
| O-1 | `WIKI_SUBJECT` is applied to two different inputs by two scripts that both claim to share one rule | Defect. See finding 3. Independent of Jev. Now the FIRST thing to fix, because it is one of the two regex failures that needs no vendor. |
| O-2 | No ground-truth labels exist for wiki-subject on any row | The bakeoff cannot report an accuracy without them. The Phase A scheme for getting them was rejected by the panel. |
| O-3 | One PostHog row carries `tv_agent` = `gemini">\n` | A seat name with quote and newline in it. Escaping defect somewhere on the write path. Not chased here. |
| O-4 | Answer quality from the wiki is still unmeasured | The handoff's finding 7. Jev's `score` is a candidate instrument. Not started, not scheduled. |
| O-5 | **Finding 4 below cites a confidence gate that the planned primitive does not have** | VERIFIED. Grok. `noul` returns only a bare number. The confidence figures in Finding 4 came from `choice`. Phase A planned to use `noul`. |
| O-6 | Codex never reviewed this | Its seat failed on a usage limit that resets 2026-09-20 14:03 ET. Dead drop at `/Users/michaelboscia/.triumvirate/dead-drop/b4048ddb-48e5-45cc-929f-e93da0b6f758-codex.md`. The panel is three seats, not four. |

---

## What changed against the handoff

**1. The first integration the handoff ranked cannot run on the data it assumed.**
The handoff ranks the wiki-subject classifier first because it "runs offline over rows already
stored." Those rows are the local SQLite ledgers, and the ledgers store **no prompt text**. An
event carries `evidence.prompt_paths` and `evidence.prompt_ids`, both already derived, plus
`required_sources`. Verified over 99 ledgers and 148 `wiki_call` events: no field holds prose.

So the current regex running over path strings is not a lazy choice. It is the only thing the
local data supports. A prose classifier at report time needs a different source.

**2. That source exists and is not blocked.** `posthog.ai_events.input` holds the prompt text, and
`/Users/michaelboscia/projects/triumvirate/scripts/wiki-controls.py` already queries it. The
handoff recorded PostHog as over quota until 2026-09-20 14:03 ET. Reads work now: 1,601
`triumvirate-daemon` `$ai_generation` rows, of which 1,464 carry substantial input text
(gemini 549, grok 455, codex 332, deepseek 128). The quota block applied to ingestion, not to
querying.

**3. The `choice` primitive does not take `options`.** It takes `criteria`, a map of option to
rubric string. The reference sheet handed over said `options` and a request built that way is
rejected with 422. `noul` takes an optional `criteria` object of `true` and `false` descriptions.
`score` takes an ordered array of at least two level descriptions.

---

## Findings from the live API

All numbers below are from real calls against `jev-1.13.0` on 2026-09-19. The probe is nine
hand-written prompts in the shape of real Triumvirate calls, five labelled wiki-subject and four
not. Nine synthetic cases is a design signal. It is not a rate and nothing here should be quoted
as one.

### Finding 1: state without domain vocabulary is the whole ballgame

Same nine cases, same questions, same model. The only difference is a short glossary prepended to
`state` naming what this codebase calls its wiki parts (bosciamem, mneme-bosciamem, pages, the
index, cards, the gold set, the labeling brief, the blind-labeler jury).

| primitive | correct without glossary | correct with glossary |
|---|---|---|
| `WIKI_SUBJECT` regex | 6 / 9 | 6 / 9 (unchanged, it reads no state) |
| `noul` | 7 / 9 | **8 / 9** |
| `choice` | 6 / 9 | 7 / 9 |

Without the glossary Jev scored "Review the blind-labeler jury output in .../gold/" at
`noul` 0.10, confidently wrong. With it, 0.92. Same for "Read LABELING-BRIEF.md and apply the
rubric": 0.41 to 0.95.

This is failure mode 1 in `https://docs.typesafe.ai/model-jaggedness/jev-1.13.md`: the model
"answers the question you wrote, not the one you meant" and "can be quite literal." It does not
know that a blind-labeler jury is wiki machinery unless the state says so. The glossary is not
prompt decoration. It is the difference between 0.10 and 0.92 on the same input.

### Finding 2: the regex and Jev fail on disjoint cases

This is the argument against a straight replacement.

- The regex misses wiki work described in prose with none of its five fitted terms present.
  "Go through the knowledge base pages one by one and check every claim still has a quote" is
  wiki work and the regex reads it as not-wiki. Jev: 0.96.
- The regex false-fires on any prompt that merely contains a path inside the wiki repo.
  "Fix the import in `.../mneme-bosciamem/ops/build_index.py`" trips `mneme-bosciamem`.
- Jev's remaining miss is that same case, at 0.80, and the ground-truth label is genuinely
  arguable: fixing the wiki's own build script may well be work on the wiki.

The jury's criticism of the regex was that it is fitted to the failures it explains. The probe
shows the shape of that fitting precisely: it is a list of proper nouns, so it matches names
rather than meaning, in both directions.

### Finding 3: the rule named "one copy" has two copies

`wiki-usage-report.py:46` imports `WIKI_SUBJECT` from `wiki-controls.py` with the comment
"so there is ONE copy of each." The pattern is shared. **The input it is applied to is not.**

| caller | what the regex is matched against |
|---|---|
| `scripts/wiki-controls.py:157` | `response_text(inp)`, the **entire prompt text** from PostHog |
| `scripts/wiki-usage-report.py:196` | `required_sources + evidence.prompt_paths`, **paths only** |

`paths_in` (`daemon/crates/triumvirate/src/wiki_usage.rs:115`) keeps only tokens starting with
`/` or `~/` and drops all prose, which its own test asserts. So a prompt that names
`mneme-bosciamem` in a sentence and points at no absolute path is wiki-subject to the controls
script and not wiki-subject to the report. One pattern, two populations, two meanings, one name.

This is the repo's signature defect shape ([[triumvirate-two-implementations]]) wearing a
comment that says it has been avoided. It should be fixed whether or not Jev is ever adopted,
and it must be fixed before the bakeoff, because "which surface does Jev replace" has no answer
until there is one surface.

### Finding 4: confidence is a usable gate, with one caveat

Across the nine glossary cases, `choice` confidence on correct answers ran 0.93 to 1.00. On its
one wrong answer it was 0.34. Before the glossary was added, though, `choice` was wrong at 0.81
and 0.79 confidence. Low confidence flags a wrong answer; high confidence does not certify a
right one, particularly when the state is missing context the model needed.

### Finding 5: the cost is not a consideration here

The nine-case probe with glossary cost 5,701 input tokens, $0.00024. Classifying all 1,464
PostHog rows with prompt text runs roughly 730k input tokens, about **$0.03**. Latency measured
at 0.24s to 0.49s round trip. Cost does not constrain any design choice in this document.

---

## Decision: direct HTTP, not a community CLI

There are at least five community CLIs and three gateways. For the Triumvirate runtime, none of
them go in. Reasons specific to this repo, not general suspicion of unofficial packages:

1. **Two implementations is this repo's top defect shape.** A CLI in the report path plus a
   client in the jury path means two prompt constructions, two glossaries, two thresholds, two
   retry policies, and a fix that lands on one of them. That is the exact failure the handoff
   names as the thing to avoid.
2. **A second key store.** `jev auth login` writes to the OS keychain. This repo already decided
   the key lives in `/Users/michaelboscia/projects/triumvirate/.env`, chmod 600, gitignored,
   and nowhere else. A keychain entry gives that rule a second surface to audit forever.
3. **Runtime drift is a live wound here.** The handoff killed 16 stale MCP server processes
   running code from two days earlier. An npm global or a `uv tool` binary updates on its own
   schedule, outside `scripts/install.sh`, against an IRON LAW that already exists about which
   binary is the real one ([[iron-law-never-run-production-from-target]]).
4. **Calibration shifts between model versions.** Our own client pins `jev-1.13.0` in source
   where a test asserts it. A CLI's default model is a flag someone forgets.
5. **There is nothing to wrap.** The API is one POST, one endpoint, one JSON body. The probe
   client in this session is 30 lines of stdlib `urllib`. The CLIs add command surfaces
   (`rerank`, `screen`, `match`, `compact`) that this repo has no use for.

**Where a CLI is genuinely the right tool:** threshold tuning and playground work on real rows,
outside the repo. `tumf/jev-cli` is the right pick for that (Python, `uv tool`, stdlib-only,
`TYPESAFE_API_KEY` takes precedence over its credential file). Install it for interactive use.
Do not let it become a dependency of anything in `daemon/` or `scripts/`.

**On `typesafe-mcp`** (giving the peers a Jev tool directly): not now, and note the specific
hazard. Grok sandboxes the MCP server process it spawns, which is finding 4 in the handoff, so a
Jev MCP tool would be reachable by some seats and not others, and the asymmetry would be silent.

---

## The plan

### Phase A: the bakeoff. Decides whether Phase B happens at all.

Offline, no daemon change, no load-bearing path touched.

1. **Fix O-1 first.** Decide the one surface `WIKI_SUBJECT` applies to and make both scripts use
   it. Recommendation: full prompt text, because it is the only surface on which a classifier of
   any kind (regex or model) can see the question being asked. This makes the report depend on
   PostHog for prompt text where today it depends on the ledger.
2. **Build the population.** The 1,464 PostHog rows with substantial `input`.
3. **Run both classifiers** over the same rows: `WIKI_SUBJECT` and Jev `noul` with the glossary
   in state, model pinned to `jev-1.13.0`. Store every Jev answer and its raw value to a JSONL
   journal as it arrives, one line per row, resumable.
4. **Label the disagreements.** Where the two agree, sample and hand-check a fixed number to
   catch shared blind spots. Where they disagree, label every case. This is the cheapest path to
   a defensible accuracy and it puts the labelling effort exactly where the information is.
   Labels are Mike's or the blind jury's, and the labeller must not see which classifier said
   what.
5. **Report both, with a threshold sweep.** The deliverable is accuracy for each, the
   disagreement set itself, and the `noul` cutoff that maximises agreement with the labels.

**Jev wins only if** it beats the regex on labelled rows the regex was never fitted to. Losing is
a real outcome and the handoff ranked this first precisely so that losing is cheap.

### Phase B: the client. Only if Phase A wins.

One client, in Rust, in the daemon crate, exposed as a `triumvirate jev` subcommand so the Python
report calls the same code path the jury would. That is how "one implementation" survives a
polyglot repo. The Phase A harness is an experiment and gets deleted, not promoted.

Requirements the client carries:
- Model pinned in source. `jev-latest` never appears in a load-bearing call.
- The glossary lives in one place with the client, not at each call site.
- Retry with exponential backoff on 429 and 529 only. 422 is a programming error and must fail
  loudly, never be retried and never be swallowed.
- Every stored verdict carries its raw value, its confidence, the model id, and the glossary
  version. A report that prints a Jev classification must say it came from a model and show the
  confidence, per the data-integrity rule. A model verdict must never read as a measured fact.
- Mutation-test the threshold. Break it on purpose and watch a test fail.

### Not in scope, and why

- **The jury verdict extractor.** Still ranked behind the classifier and still fallback-only for
  the `None` case, per the handoff. A vote path that depends on a paid network call is a new
  failure mode on the surface this repo cares about most.
- **Seat and depth routing.** Changes dispatch behaviour. Waits for a measured track record.
- **The sight gate.** Never. It is mechanical on purpose and a probabilistic judgement there
  would undo the property that makes it trustworthy.
- **Call-time classification.** Rejected. It would put a network call in the ask path and would
  destroy the property that makes the bakeoff possible: that classification happens at read time,
  so two classifiers can be run over the same stored rows and compared.

---

## Peer panel findings, 2026-09-19

Three seats, one lens each, artifacts frozen before dispatch. Grok ran with `grok_depth: deep`,
`require_sight: true` and both briefs as `required_sources`, and made 25 tool calls. Antigravity
made 2. DeepSeek has no filesystem access so its inputs were pasted. Codex failed on quota (O-6).

### Finding 1 (Grok, VERIFIED here): the glossary inverts the measurement

The glossary that produced the 0.10 to 0.92 result re-teaches the exact vocabulary that
`WIKI_SUBJECT` was surgically cut to avoid. From `scripts/wiki-controls.py`:

> NEVER let a term of this rule appear in the map itself. The first version included
> `bosciamem wiki`, and the map's first line is `# bosciamem wiki index`: the moment the map is
> delivered through the prompt, every delivered call would classify as wiki-subject and the
> entire delivered arm would vanish from the measurement. Grok found it in review.

Checked directly against `/Users/michaelboscia/projects/mneme-bosciamem/_index.md`:

| | Result |
|---|---|
| Glossary terms appearing in the map | **7 of 13** |
| Map line 1 | `# bosciamem wiki index (20 pages, 4712 evidence cards, ...)` contains `bosciamem`, `pages`, `index`, `cards` |
| Map line 3 | contains `claims`, `verbatim quote`, `source note id` |
| Existing `WIKI_SUBJECT` vs the map | **No match, by design.** `main()` refuses to run if it ever matches |

Every delivered-arm prompt carries the map. A meaning classifier taught that this vocabulary
means "wiki" will classify every one of them as wiki-subject, and the delivered arm, the thing
being measured, disappears. The same defect, found by the same reviewer, recreated by the fix.

Grok's framing: wiki-subject "is not a product feature, it is a partition of a measurement," so a
better eval "would just measure the wrong task more carefully."

Grok also notes the plan string-concatenates the glossary onto the prompt, which is the worst
possible boundary, while the API accepts `state` as an **object with named fields**. If any
glossary survives, it belongs in a separate field, never concatenated into the text being judged.

### Finding 2 (Grok, VERIFIED): the cited confidence gate does not exist on `noul`

Finding 4 above argues confidence is a usable gate. Those numbers are `choice.confidence`. A
`noul` answer is `{"type": "noul", "noul": 0.41}` and carries no confidence field at all. Phase A
planned to use `noul`. The gate was cited for a primitive that does not have one.

### Finding 3 (DeepSeek): the missing arm is regex plus glossary

The headline result is that adding a glossary moved Jev from 7/9 to 8/9. The arm never run is the
one that decides the question: the **incumbent regex, improved**.

> The glossary worked because it names the project's jargon, and a 5-term regex exists precisely
> because someone already tried to encode jargon as terms and stopped at five. The glossary
> result is evidence that the term list is too short, not that a model is needed.

Supported by the probe's own data. The regex had exactly two failure types, and both have a free,
deterministic fix: the misses are term coverage (add terms), and the false fire is a bare path
mention, which is O-1, the two-surface bug. Neither needs a vendor.

### Finding 4 (DeepSeek): no Phase A outcome licenses an action

Win, and what is won is a more accurate exclusion filter feeding a usage count. Lose, and the
regex is kept. Both outcomes are "no change worth making," while the two candidates that would
change something (a silently lost vote, the missing quality instrument) are deferred for being
load-bearing. Named pattern: **pilots get chosen for completability, not for consequence.**

DeepSeek also observes the pilot measures argmax accuracy on binary classification, discarding
every property Jev is sold on, and that the documented `P(noul) + P(not noul) != 1` means the
probabilities cannot be inverted or combined arithmetically anyway.

### Finding 5 (Grok and DeepSeek, converging): disagreement-only labelling is rigged

It estimates the margin between two classifiers, not absolute accuracy, and it is structurally
blind to rows where both are wrong the same way. Demonstrated with this repo's own probe data:
"Fix the import in `.../mneme-bosciamem/ops/build_index.py`" is a shared false positive. The regex
fires, Jev says 0.80. **They agree, so the row never enters the disagreement set.**

Grok: "The labelled rows are selected by the regex. That is not a held-out population. It is the
residual of the fitting." This shop already has the rule, from the wiki measurement work:
agreement between similar instruments confirms consistency, not correctness.

### Finding 6 (Grok and DeepSeek, converging): the injection surface is the measurement

Jev is documented as steerable by content in `state`. Here `state` would be prompts and replies
written by other AI agents, often containing the map because this repo puts it there. DeepSeek
adds the incentive case: an agent could influence whether its own calls are excluded from the
usage statistics it is measured by. A steerable metric is worse than an inaccurate one, and the
regex cannot be talked into anything.

### Finding 7 (Antigravity): depend on nothing until the wire format settles

Backs writing the Rust client, and goes further than the survey: an unofficial wrapper of an
official SDK adds a failure point rather than removing one, and an ecosystem where two CLIs
already disagree on the wire format warrants holding at raw HTTP. Wants `typesafe-ai` on PyPI
explicitly banned and hash-pinned lockfiles anywhere Python touches this.

Correction: this seat claimed the wiring brief supersedes the survey on client choice. The order
is the reverse. Its argument does not depend on that, but the seat had it backwards, and it made
only 2 tool calls.

### What the panel did not dispute

Direct HTTP over any community client. Writing the Rust client rather than adopting a 0.1.x
crate. Pinning `jev-1.13.0`. Retrying 429 and 529 only and failing loudly on 422. Keeping Jev out
of the sight gate.

### The revised first move

Not a Jev bakeoff. Fix O-1, extend the regex from reading real rows, and re-measure. If that
lands where Jev lands, there is no adoption case for this use at any accuracy. That costs no
vendor, no network call, no pinned version, and no contaminated measurement.

Jev is not ruled out. The question moves to the candidates deferred for being load-bearing, which
is where the panel's reasoning and the original finding both point.

## Verified API facts

| Fact | Value |
|---|---|
| Endpoint | `POST https://api.typesafe.ai/v1/systemone` |
| Auth | `Authorization: Bearer $TYPESAFE_API_KEY` |
| Request | `state`, `model`, `questions` |
| `noul` question | `type`, `instructions`, optional `criteria` {`true`, `false`} |
| `choice` question | `type`, `instructions`, **`criteria`** map of option to rubric. Not `options`. |
| `score` question | `type`, `instructions`, `criteria` ordered array, two levels minimum |
| `noul` answer | `noul`, a number 0 to 1. No confidence field. |
| `choice` answer | `choice`, `probabilities` map, `confidence` 0 to 1 |
| Response | `model`, `answers`, `usage` {`input_tokens`, `output_tokens`} |
| Errors | 401 bad key, 422 validation, 429 rate limit, 529 overloaded |
| Measured latency | 0.24s to 0.49s round trip, this machine, 2026-09-19 |
| Measured cost | 5,701 input tokens = $0.00024 |

Not stated anywhere in the docs read: max questions per request, max state size, request
timeouts, numeric rate limits, and whether requests are logged or used for training. Treat all
five as unknown.

Documented failure modes that bear on this repo: literal reading, weakness on counting and
arithmetic, weakness on date ordering, degradation as irrelevant state grows, susceptibility to
adversarial content in state, and no guarantee that `P(noul)` and `P(not noul)` sum to 1 across
separate questions.
