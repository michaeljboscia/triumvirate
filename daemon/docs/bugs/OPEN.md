# Open defects

**What this is:** the single list of things currently broken in Triumvirate. One line per
defect, with the evidence that proves it and the check that would close it.

**Why it exists:** on 2026-05-25 we correctly diagnosed a defect, wrote "this is *the* first
patch" next to it, and did not apply it for two months. It then caused a second incident and
a retracted claim. Individual bug reports capture an incident well and then go quiet. This
file is the thing you read to answer "what do we know is broken right now."

**Rules:**
- A defect leaves this list only when the CHECK column passes, and the row moves to Closed
  with the date and what fixed it.
- "We think it's fine now" is not a close. Run the check.
- If a check cannot be run, say so in the row. An unverifiable defect stays open.
- Last reviewed date goes at the bottom. If it is stale, the list is not being used.

---

## Open

### D-025 - `u_grok_06` fails intermittently in a full-suite run
**Found:** 2026-09-20 · **Severity:** LOW · **NOT FIXED**
**Evidence:** `cargo test -p mcp-bridge --lib` failed once on
`tests::u_grok_06_command_resolves_to_the_grok_binary_by_default`. The same test passes when run
alone, and the same full suite then passed on three consecutive runs (185 tests each). One failure
in roughly five full runs.
**Not caused by the change it appeared during.** It first showed while D-020/D-022/D-023 were in
the tree, so the obvious suspicion was the six tests added alongside them. Those tests touch no
environment variable and no process-global state. The pristine tree was re-tested to confirm the
suite was green without them, which it was, so the added tests changed test SCHEDULING and exposed
something already there rather than introducing it.
**Suspected mechanism, NOT confirmed:** the test takes `env_lock()` with
`.expect("env lock poisoned")` while the test immediately below it takes the same lock with
`.unwrap_or_else(|e| e.into_inner())`, which tolerates poisoning. A sibling panicking while holding
that lock would fail this test and not its neighbour. Which sibling, and whether poisoning is
actually the mechanism, has NOT been established. Do not write the fix from this paragraph.
**Why it is filed rather than shrugged off:** an intermittently red suite teaches people to re-run
until green, which is how a real regression gets waved through. This repo has hit process-global
test state before.
**Check:** the cause is identified, and `cargo test -p mcp-bridge --lib` passes 20 consecutive full
runs with no re-runs.

### D-020 - DeepSeek metered cost is computed from an ASSUMED model
**Found:** 2026-09-20 · **Severity:** HIGH · **CHECK PASSED 2026-09-20. RESOLVED, see Resolution at the end of this row.**
**Evidence:** 127 of 142 deepseek rows carry `$ai_model = unknown`, `tv_billing = metered`, and a real
dollar figure (`sum($ai_total_cost_usd) = 0.1781` over 98,900 input tokens). Measured in PostHog
2026-09-20 over `distinct_id = 'triumvirate-daemon'`, 180 days.
**Root cause:** `daemon/crates/mcp-bridge/src/posthog.rs:341`, `billing_for` reads
`model.unwrap_or("deepseek-v4-flash")`. An ABSENT model silently becomes the CHEAPEST model.
`deepseek-v4-pro` is 3.1x flash on both input (0.435 vs 0.14) and output (0.87 vs 0.28), and the
15 rows that DO name a model are all v4-pro. So the priced-by-assumption set is 4x the token
volume of the only set we can price honestly, and it is priced at the floor.
**Why this is the worst of the five:** the function's own doc comment states the rule it breaks:
"If a model is not listed here we return UnknownPrice and emit no cost, a wrong cost is worse than
no cost." The `_ => Billing::UnknownPrice` arm honours that for an UNRECOGNISED model. The
`unwrap_or` bypasses it for an ABSENT one. A missing cost reads as a gap; a wrong cost reads as a
fact, and nothing downstream can tell it was guessed.
**Check:** `billing_for("deepseek", None)` returns `UnknownPrice`, and a deepseek generation with no
model emits NO `$ai_total_cost_usd`. Mutation: restore the `unwrap_or` default and the test goes red.

**Resolution (`c1526bf`):** the deepseek arm no longer defaults an absent model. `None` returns
`UnknownPrice` and emits no cost, which is what this function's own doc comment already required.
**CHECK PASSED 2026-09-20:** `billing_for("deepseek", None)` returns `UnknownPrice` and
`cost_usd("deepseek", None, ..)` returns `(None, "unknown")`, asserted by
`an_absent_deepseek_model_is_not_priced_at_the_floor`. Mutation: restoring the floor default turns
that test red, confirmed by running it.
**Live consequence, observed 2026-09-20 09:42 ET:** a deepseek call now carries
`$ai_model = deepseek-v4-flash` and `$ai_total_cost_usd = 4.82384e-05` computed from the model that
actually ran, not from a default. The refusal path is the safety net, not the normal case, because
D-021 gave this seat a real model source in the same session.
**Note on what was fixed:** the pre-existing test `deepseek_flash_is_priced_from_the_published_table`
was holding this defect in place. It called `cost_usd("deepseek", None, ..)` and asserted flash
pricing, so its name claimed it pinned published prices while its assertion blessed the default.
That is the D-004 shape. It now names the model explicitly.

### D-021 - `$ai_model` is `unknown` on 63% of rows; set_model is gated to one seat
**Found:** 2026-09-20 · **Severity:** HIGH · **CHECK PASSED 2026-09-20 16:34 ET. ALL FOUR SEATS FIXED and verified live. RESOLVED.**
**Fix landed (`407001a`):** the `if agent == "gemini"` gate is gone, so any connector that knows its
model reports it. deepseek additionally fills `cli_version` with the model it resolved and sent.
**VERIFIED LIVE, deepseek:** 2026-09-20 09:42 ET, daemon pid 46068 on the freshly installed binary, a deepseek generation carried
`$ai_model = deepseek-v4-flash` where every unattributed row before it said `unknown`.
**CHECK RUN 2026-09-20 14:04 ET, codex: IT FAILED, and the expectation recorded here was wrong.**
The call succeeded (6,612ms, a real answer) and still charted `$ai_model = unknown`. Ungating
`set_model` did NOT fix codex.

**Why, established by capture rather than by reading code:**
1. `codex_protocol()` defaults to `"exec"` and `TRIUMVIRATE_CODEX_PROTOCOL` is not set on this
   daemon, so the live parser is `CodexExecParser`, which hardcodes `cli_version: None`
   (`crates/agent-adapter/src/codex.rs:769`). `CodexAppServerParser`, the one that reads
   `result.model`, never runs.
2. The exec stream does not carry the model AT ALL. A real capture from the installed
   codex-cli 0.154.0 (`codex exec --json`, 2026-09-20) is five events:
   `thread.started` (thread_id only), `turn.started`, two `item.completed`, and `turn.completed`
   carrying `usage` alone. No model field anywhere in the stream.
3. The app-server escape hatch does not exist on this version. `codex app-server --help` on
   0.154.0 is a tooling NAMESPACE (`daemon`, `proxy`, `generate-ts`, `generate-json-schema`),
   not the JSON-RPC-over-stdio server `CodexAppServerParser` was written against. This is exactly
   what `crates/mcp-bridge/src/codex_capabilities.rs` predicted for 0.121+.

**So `CodexAppServerParser`'s model capture is effectively dead code against the installed CLI,**
and its only test asserts a HAND-WRITTEN payload (`codex_app_server.rs:233`,
`{"result":{"model":"codex-app-server"}}`), not a real capture. That is why it read as a working
source: a green test on an invented payload for a protocol shape that no longer exists. The grok
half of this defect was fixed from REAL fixtures for exactly this reason.

**Rejected, and why:** `--model` / `-c model=` are INPUT flags, and `~/.codex/config.toml` is
configuration. Both say what we asked for, not what served the turn. Using either would be D-020's
defect, an absent value replaced by a plausible one, and it would be worse here than a blank,
because codex is the heaviest seat on the board and the number would look authoritative.

**THE PARAGRAPH ABOVE WAS WRONG, and it is left in place deliberately.** It concluded "codex cannot
report the served model" from checking exactly two surfaces and generalising. Mike rejected the
conclusion. He was right.

**CHECK PASSED 2026-09-20 16:34 ET (`d985f8f`).** A live codex generation carried
`$ai_model = gpt-5.6-sol`, `$ai_provider = openai`, `tv_billing = subscription`, cost `0.0`, over
23,539 input tokens. The heaviest seat on the board is now fully attributed.

**Where the model actually lives:** codex writes a rollout file per thread under
`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<thread_id>.jsonl`, carrying a `turn_context` record
per turn:

```json
{"type":"turn_context","payload":{"turn_id":"...","model":"gpt-5.6-sol","cwd":"..."}}
```

The daemon already held the key: `CodexExecParser` stores `thread.started`'s `thread_id` as
`session_id`. Verified on a DAEMON-driven call before any code was written, not a hand-run one.
Read by `mcp_bridge::codex_rollout::model_for_session`, which lives in mcp-bridge rather than the
parser because it is filesystem I/O and the parsers are pure over their stream, and which returns
`None` on every failure so it can never fail a call that already succeeded.

**Still not `--model` / `-c model=` / `config.toml`.** Those remain what we ASKED for. The rollout
is what codex recorded after the fact.

**The lesson this row is really about.** Codex model attribution has now been got wrong twice, and
both times by reading source instead of measuring. The first attempt trusted
`CodexAppServerParser` because its test was green; that test asserts a HAND-WRITTEN payload
(`{"result":{"model":"codex-app-server"}}`) for a protocol shape the installed CLI no longer
speaks, so it was green and meaningless. The second attempt declared the model unobtainable after
two probes. The grok half went right because its fixtures are REAL captures, which is also how the
1.0.13 to 1.0.30 drift check was possible. Every test added for this fix is built from a verbatim
real rollout line for that reason.

**The first version of this fix was itself a two-surface defect (`d985f8f`, corrected in
`81abb87`).** It resolved the rollout model inside the telemetry guard. Two consumers read
`parsed.cli_version`, and the other is `persist_daemon_token_record`, which writes the local
token-economics ledger and runs THIRTY LINES EARLIER in the same function (a third read sits in the
rejected-no-sight arm, earlier still). So PostHog got `gpt-5.6-sol` and the ledger kept writing an
empty model for the same call, from the same field. It is now resolved at the codex connector,
above every consumer, and the telemetry guard's codex special case is deleted because the generic
path covers it.

**VERIFIED ON ALL THREE SURFACES, 2026-09-20 16:43 ET, one codex call, daemon pid 7612:**

| Surface | Value |
|---|---|
| `token-economics.db` | `codex \| gpt-5.6-sol \| 01a0c08e-e7d7-78d0-8e23-8c948193c72d \| 23539` |
| daemon span | `agent.model = gpt-5.6-sol` |
| PostHog `$ai_generation` | `$ai_model = gpt-5.6-sol`, provider `openai`, billing `subscription` |

Checking only the surface just fixed is how the first version passed. A fix closes on its own
check, and the check has to cover every surface the value reaches.

**Related, NOT merged, recorded so nobody "consolidates" it by accident:** `token-economics`
already scans `~/.codex/sessions` and extracts a model, via a recursive search for any key named
`model`/`model_name`/`modelId` across three agents' formats
(`scanner.rs::extract_model`). That is deliberately loose because it reconstructs historical spend
from whatever it finds. `codex_rollout::model_for_session` is deliberately strict: `turn_context`
records only, newest wins. Two readers, two jobs. Merging them would make one of them wrong.

**And one of those new tests had the same defect.** The hostile-session-id test stayed GREEN when
the path validation was deleted, because those ids match no file either way: it checked the holder,
not the guard. Mutation testing caught it. It now plants a rollout that a non-id WOULD resolve to,
so the assertion measures the guard. A test that cannot fail is not evidence, whoever wrote it.
**FIXED AND VERIFIED LIVE, grok (`3a3ddd5`):** at 09:42 grok still returned `unknown`; it now does
not. The CLI had been reporting the answer on every single turn and nothing read it. grok's `end`
event carries a `modelUsage` map KEYED BY MODEL NAME, present in all four real 1.0.13 fixtures
already committed here, and `grep -rn modelUsage` over the workspace returned nothing.
`GrokStreamParser::finish()` returned a hardcoded `cli_version: None` while the value sat one field
away in an event it was already parsing for `total_cost_usd`.
**CHECK PASSED 2026-09-20 11:11:43 ET**, daemon pid 64103: a live grok generation carried
`$ai_model = grok-4.6-build`, `$ai_provider = x-ai`, `tv_billing = subscription`, cost `0.0`, over
12,646 input tokens. Every attribution field on the heaviest subscription seat is now populated.
**Deliberately NOT `grok_model()`,** which the previous handoff nominated as the candidate source.
It reads `TRIUMVIRATE_GROK_MODEL`: the model we ASKED for, empty by default, so it reports intent
rather than fact and says nothing at all in the common case. Charting intent as fact is D-020's
class of defect.
**Version drift was checked, not assumed.** The fixtures are grok 1.0.13 from 2026-08-30; the
installed binary is 1.0.30. A fix validated only against old captures is a fix against a format
nobody runs, and it fails silently back to "unknown" with every test green. A real capture from
1.0.30 was taken 2026-09-20 and committed as
`daemon/crates/agent-adapter/tests/fixtures/grok-streaming-1.0.30-20260920.jsonl`. That fixture is
the drift alarm for the next grok release.
**Regression check for the next codex upgrade:** if the rollout format changes, `$ai_model` returns
to `unknown` SILENTLY, because `model_for_session` degrades to `None` by design. After any codex
upgrade, run one codex call and confirm `$ai_model` in PostHog is not `unknown`. That is the one
failure mode this fix cannot announce on its own.

**Original diagnosis, kept for the record:**
**Evidence:** 1,062 of 1,690 rows cannot say which model answered. By seat: codex 399 of 399
(100%), grok 535 of 535 (100%), deepseek 127 of 142 (89%), gemini 69 of 613 (11%).
**Scale note that reorders the priority:** codex is the HEAVIEST seat on the board by input tokens
(64.5M, against grok 27.6M and gemini 16.6M) and not one of its rows names a model. The
2026-09-20 handoff ranked this behind the two grok match arms; by token volume it is the largest
hole in the dataset.
**Root cause:** `daemon/crates/triumvirate/src/agent_exec.rs:1135` reads `if agent == "gemini"`.
`set_model` has exactly ONE call site in the workspace and it sits inside that branch. Every other
seat falls back to the agent key, which cannot answer "which model ran".
**Known non-answer:** `grok_model()` at `daemon/crates/mcp-bridge/src/grok.rs:115` reads an env var
rather than parsing what the CLI actually used, so it reports intent, not fact. Each seat needs its
own source for the model it really ran.
**Coupling:** while this is open, D-020 cannot be closed by pricing correctly, only by refusing to
price. Attribution has to land before metered cost can be trusted.
**Check:** a codex, grok and deepseek generation each carry a non-`unknown` `$ai_model` in PostHog.

### D-022 - `$ai_provider` is `unknown` for grok AND claude
**Found:** 2026-09-20 · **Severity:** MEDIUM · **CHECK PASSED 2026-09-20. RESOLVED, see Resolution at the end of this row.**
**Evidence:** 536 rows report `$ai_provider = unknown` (535 grok plus the one malformed row of D-023).
**Root cause:** `daemon/crates/mcp-bridge/src/posthog.rs:407`, `provider_for` matches `gemini`,
`codex`, `deepseek` and falls through `_ => "unknown"`. There is no `grok` arm.
**Wider than the handoff recorded:** `claude` is ALSO absent. `supported_agent_names()` returns
`["gemini", "codex", "deepseek", "claude", "grok"]`, so TWO of five supported seats map to
"unknown". `claude` shows no rows today only because that seat has not been exercised, which means
this defect would have appeared later as a second surprise rather than being fixed once.
**Check:** for EVERY agent in `supported_agent_names()`, `provider_for` returns a value that is not
"unknown". Mutation: delete any one arm and the test goes red.

**Resolution (`c1526bf`):** `grok => "x-ai"` and `claude => "anthropic"` added. Slugs are
OpenRouter's, because PostHog matches `$ai_provider` + `$ai_model` against OpenRouter pricing first
(posthog.com/docs/ai-observability/calculating-costs, checked 2026-09-20).
**CHECK PASSED 2026-09-20 09:42 ET:** a live grok generation carried `$ai_provider = x-ai` where
every grok row before it said `unknown`. `every_supported_agent_has_a_provider` walks
`supported_agent_names()` so a sixth seat cannot be added without answering this. Mutation:
deleting the grok arm turns it red, confirmed by running it.

### D-023 - `tv_billing` is `unknown` for grok, and cost is omitted
**Found:** 2026-09-20 · **Severity:** MEDIUM · **CHECK PASSED 2026-09-20. RESOLVED, see Resolution at the end of this row.**
**Evidence:** 535 grok rows carry `tv_billing = unknown` and NO `$ai_total_cost_usd`, across 27.6M
input and 3.6M output tokens. The heaviest-but-one seat is absent from every cost and billing view.
**Root cause:** `daemon/crates/mcp-bridge/src/posthog.rs:336`, `billing_for` matches
`"codex" | "claude" | "gemini" => Subscription`, then `deepseek`, then `_ => UnknownPrice`. No `grok`
arm.
**The correct value is known, not unknowable:** grok runs on a SuperGrok subscription, so
`tv_billing` is `subscription` and the marginal cost is a real `0.0`. Reporting `unknown` says "we
could not price it" when the truth is "it is free", which is a different and worse claim.
**Same shape as D-022:** both are a missing arm in a match that enumerates agents, and both were
introduced by adding a seat without a test that walks the canonical list.
**Check:** for EVERY agent in `supported_agent_names()`, `billing_for(agent, None)` is not
`UnknownPrice`. Mutation: delete any one arm and the test goes red.

**Resolution (`c1526bf`):** `grok` added to the subscription arm.
**CHECK PASSED 2026-09-20 09:42 ET:** a live grok generation carried `tv_billing = subscription`
and `$ai_total_cost_usd = 0.0` across 12,529 input tokens, where every grok row before it said
`unknown` and carried no cost at all. Mutation: removing grok from the arm turns both
`grok_is_a_subscription_seat_priced_at_a_real_zero` and
`every_supported_agent_has_a_billing_classification` red, confirmed by running them.

### D-024 - One row carries a seat name containing a quote and a newline
**Found:** 2026-09-20 · **Severity:** LOW · **ROOT-CAUSED AND CHECK PASSED 2026-09-20. RESOLVED, see Resolution at the end of this row.**
**The hypothesis in this row was WRONG.** It guessed "an escaping defect somewhere on the write
path". There is no escaping defect. `CallTelemetry::new` is built from the RAW request string at
`agent_exec.rs:593`; `is_supported_agent` rejects at 598; `tel.set_agent(normalized)` sits at 615
and is never reached. The guard emitted on drop carrying exactly what the caller sent, and
`tv_agent_display = Gemini">` confirms it: `display_agent_name` ran on an already-malformed value
and fell through to its generic capitaliser. The write path worked. It charted, faithfully, the
garbage it was handed.
**Evidence:** one row has `tv_agent` = `gemini">\n` (literal `">` then a newline). It also reports
`$ai_input_tokens = 0`, `$ai_output_tokens = 0`, and unknown on model, provider and billing, so it
is inert in every aggregate except as a phantom sixth seat in a `GROUP BY tv_agent`.
**Why it is ranked last and still filed:** it is one row, but the shape (`">` plus a newline)
suggests something interpolating an unescaped value into a quoted string on the write path. If that
path is shared, other fields on other rows may be affected without being obvious, because only a
value that happens to contain a quote reveals it.
**Open question, not an assumption:** whether the mangling happens at the call site that names the
agent, in the telemetry struct, or in JSON assembly. None has been ruled out.
**Check:** the source of the malformed value is identified in code, and a test feeds a
quote-and-newline-bearing agent name through the write path and asserts the emitted `tv_agent` is
either clean or rejected. Until then this row stays open.

**Resolution (`3e29e79`):** `chart_agent` bounds `tv_agent` to `supported_agent_names()` plus the
single sentinel `"unsupported"`, applied at `ai_generation_props` so BOTH emitting surfaces are
covered rather than only the one that produced the known row. Aliases still resolve to canonical
keys, so supergrok/agy/antigravity traffic lands on its real seat. The rejected name is not
shipped: it is already in the error returned to the caller and in the daemon logs, which is where
an unbounded string belongs.
**CHECK PASSED 2026-09-20 09:42:59 ET:** the exact 2026-08-26 value (`gemini">` plus a newline) was
sent live to `ask_agent`. It was rejected pre-dispatch and its generation charted as
`tv_agent = unsupported` / `tv_agent_display = Unsupported`, with the diagnostic preserved in
`$ai_error` ("ask_agent supports only: gemini, codex, deepseek, claude, grok"). No quote and no
newline reached the dimension. Mutation: bypassing `chart_agent` at the emit site turns
`the_malformed_seat_name_from_2026_08_26_does_not_chart_as_an_agent` red; dropping normalization
inside it turns `bounding_preserves_every_real_seat_and_its_aliases` red. Both confirmed by running
them.
**The real defect class, for the next person:** `tv_agent` was an unbounded caller-controlled CHART
DIMENSION. Any caller could mint a phantom seat that appears in every `GROUP BY tv_agent` from then
on, and ship arbitrary caller text to a SaaS. This file already applies that reasoning to
`repo_name` ("cardinality garbage AND would leak the operator's home directory"); it had never been
applied to the agent name. Check any other caller-supplied value that becomes a PostHog dimension.

### D-004 - Failed generations carry no error text
**Found:** 2026-07-28 · **Severity:** MEDIUM · **FIX LANDED 2026-09-19; live confirmation blocked on the quota reset**
**Evidence (original):** failed `$ai_generation` events of 2026-07-28 and 2026-08-06 carried
`tv_outcome` and nothing else; `$ai_error` did not exist in this project's taxonomy. The outcome
half was resolved 2026-08-07 (`cancelled` instead of `unreported`).
**What was actually wrong (found 2026-09-19):** the cause WAS captured. `CallTelemetry::failure`
stored it in `self.detail`, and it was then sent only to a separate `$exception` event: the
`AiGeneration` struct had no field for it, so the generation itself said "error" and never said
what. An existing test claimed "the rejection is inspectable in the generation" and asserted
`t.detail.is_some()`, which checked the holder and passed while the event never carried it.
**Fixed:** `AiGeneration` carries `error`, emitted as PostHog's documented `$ai_error` ("the error
message or object", confirmed against posthog.com/docs/ai-observability/generations) on every
error outcome, and as `tv_detail` whenever present. BY CONSTRUCTION an event marked
`$ai_is_error: true` now always carries a non-empty `$ai_error`: a caller-side cancel or an
unclassified exit says so in words rather than shipping silence. Covered on BOTH surfaces that
emit failed generations, the telemetry guard and `record_dispatch_generation`; fixing only the
first would have left every failed dispatch causeless. The misleading test now asserts on the
emitted payload. Removing the fallback cause turns `every_error_generation_carries_a_non_empty_cause` red.
**Why it is not closed:** the check is "a failed generation IN POSTHOG carries a cause string",
and nothing lands in PostHog while the account is over quota (D-018 measured it). The payload
is proven; its arrival is not, and closing on the payload would be closing on reasoning.
**Unblocks:** 2026-09-20 14:03 ET.
**Check:** with `/health` reporting `telemetry_delivery: trusted`, force one failed dispatch and
find its `$ai_error` in PostHog.

**CHECK PASSED 2026-09-20 09:42:38 ET. RESOLVED.** A live `ask_agent` to codex failed on quota and
its `$ai_generation` arrived in PostHog carrying the full cause in `$ai_error`: the complete
three-attempt faildown chain, "codex attempt 1/3: codex connector failed: exited with status exit
status: 1; codex said: You've hit your usage limit ... -> codex attempt 2/3 ... -> codex attempt
3/3 ...". Not forced: it was a genuine failure encountered while verifying D-021.
**On the `/health` precondition:** this row waited on `telemetry_delivery: trusted` as a proxy for
"events are arriving". The row's own arrival is the direct measurement that proxy stands in for, so
the check is satisfied in substance by stronger evidence than it asked for.
**The blocker recorded here was wrong.** This row and D-005 both said nothing lands in PostHog
until the quota reset at 2026-09-20 14:03 ET. Ingestion was measured healthy at 09:38 ET that
morning: latest event 09:38:47, 701 events in the preceding hour, 19,185 in 12 hours. The 14:03
reset belongs to CODEX's usage quota, which is a different account and a different limit. Two
unrelated quotas were conflated, and a defect sat blocked on a condition that had already cleared.

### D-005 - Instrumentation streams gone silent, cause unknown
**Found:** 2026-07-28 · **Severity:** LOW · **Blocked on an external reset, NOT fixed**
**Remaining streams:** `tv_review_verdict`, `tv_review_requested`, `tv_fleet_spawn`,
`tv_maintenance`. (`tv_codex_dispatch` was shown healthy on 2026-08-02.)
**What changed 2026-09-19:** this row could never be resolved because nothing could tell "path
idle" from "emitter broken". That question now has an instrument. The delivery round trip
(`mcp_bridge::telemetry_delivery`, D-018) reports whether events are arriving AT ALL, so a
quiet stream is interpretable: while `/health` says `telemetry_delivery: untrusted`, no stream's
silence means anything; while it says `trusted`, a silent stream is an idle path.
**Why it is still open:** its own check is "exercise each path once and confirm the event
lands", and nothing lands while the PostHog account is over quota. Closing it on the new
instrument alone would be closing it on reasoning rather than on its check, which is exactly
what this file forbids.
**Unblocks:** NOTHING. This row is no longer blocked, as of 2026-09-20.
**The blocker was wrong, same error as D-004.** PostHog ingestion was measured healthy at 09:38 ET
on 2026-09-20 (latest event 09:38:47, 701 in the preceding hour) and `$ai_generation` events from
this session landed within seconds. The 14:03 reset is CODEX's usage quota, an unrelated account
and limit. This row can be worked whenever someone chooses to.
**Check (unchanged, and now runnable):** exercise `tv_review_verdict`, `tv_review_requested`,
`tv_fleet_spawn` and `tv_maintenance` once each and find each event in PostHog. NOT done in this
session: the telemetry work here covered `$ai_generation` only, and claiming these four by
association would be exactly the "we think it's fine now" close this file forbids.

### 2026-09-19: the test suite wrote wiki_call events into real shared ledgers (D-019)

Found by the first real run of `scripts/wiki-usage-report.py`: 11 rows said "wiki not loadable
at call time" although the real wiki loaded fine on every live call. Traced to my own
`cargo test` run. The suite drives `execute_ask_agent` with stand-in agents, some of its calls
use shared directories (`/private/tmp/project`, `/private/tmp/worker-reuse`, and with no cwd this
crate's git-tracked `.triumvirate/ledger.db`), and step one's recorder wrote a `wiki_call` event
for each. The report counted them as peer calls. A test that clears HOME also made the wiki path
relative, which is the "not loadable" text.

Fixed three ways: a test build records only when a test opts in
(`TRIUMVIRATE_TEST_RECORD_WIKI_CALL`, set by the four `wiki_call_*` tests alone); every
test-build event carries `evidence.harness = "cargo-test"`, which the report holds apart; and
`wiki_dir()` refuses a relative path. The 11 pre-fix rows stay in place, held apart as
unloadable.

**CHECK PASSED, 2026-09-19:** full `cargo test -p triumvirate --bin triumvirate` (296 passed), then
every ledger under /private/tmp and ~/projects scanned for `wiki_call` rows newer than the run's
start: zero. Negative control: the same run with the guard removed wrote 12 rows into six shared
ledgers, so the scan can see them. Mutants for the harness mark and the absolute-path guard each
fail a test.

### 2026-09-19: agy ran past its version pin on every dispatch (D-007)
The pin was 1.1.5 in `~/.claude.json` and 1.0.2 as the code default, while 1.2.7 was installed
and serving every call, so the mismatch warning fired on every agy dispatch. A warning that
fires unconditionally is furniture: nobody acts on it, and it trains the reader to skip the
line where a real mismatch would one day appear.
Owner's decision (2026-09-19): move the pin to what is installed and stay warn-only. Set to
1.2.7 in both places, and the doc comment that called the default "the last version verified
against the live binary" was corrected, because it was no longer true.
**Recorded plainly, so the pin is not read as more than it is:** the REQ-060-064 verification
battery was NOT re-run for 1.2.7. This pin now means "the version we run", not "a validated
version". Making it mean the second requires running that battery and updating the comment.
**CHECK PASSED:** the daemon restarted with `TRIUMVIRATE_AGY_EXPECTED_VERSION=1.2.7` against
an installed 1.2.7 emits no version-mismatch warning.


### 2026-09-19: agy quota detectors matched benign glog noise (D-014)
`classify_failure_message` matched ANY occurrence of `429` and ANY occurrence of `quota`, so a
glog thread id containing `429` and a health line containing `doRefreshQuota` both classified
as capacity/quota. A false quota is not free: it triggers the retry backoff AND feeds the
circuit breaker, so benign log noise could back off a healthy call and nudge the breaker
toward shedding real traffic.

Fixed by a Codex worker in an isolated worktree (task `d014-agy-quota-anchors`, commit
22db81d, cherry-picked as 58766d2). Both detectors now go through
`contains_anchored_phrase`, which requires the match not to be embedded in an identifier or a
longer numeric token, over the markers `RESOURCE_EXHAUSTED`, `quota exceeded`, `code 429`,
`HTTP 429`, `(429)` and `capacity`.

**CHECK PASSED:** `quota_detectors_ignore_benign_glog_tokens` classifies the real 2026-08-20
thread-id line and a `doRefreshQuota` line as `AuthOrExec`, and
`quota_detectors_preserve_anchored_positive_signals` keeps every real quota form classifying
as `Quota`. Verified independently of the worker's own report: I ran both tests, then reverted
the anchoring by hand and confirmed the negative test goes red.


### 2026-09-19: the daemon had no shutdown path at all (D-001)
Fourteen months of `tv_daemon_started` with no matching stop, because `axum::serve` was called
with no `with_graceful_shutdown` and no signal handler existed anywhere. SIGTERM killed the
process outright, so there was nothing to emit and nothing to log. A crash and a clean restart
were the same evidence, which is the exact case the defect dashboard was built to catch.

Added `await_shutdown_signal`, which resolves on SIGTERM or SIGINT and NAMES which one:
"the daemon stopped" is half an answer, and a row that cannot tell an operator restart from a
person at a terminal cannot tell a deployment from an interruption. SIGKILL is deliberately
absent and cannot be caught, so an exit with no line still means something specific: killed
outright or died, which should read differently from a clean stop.

The LOG LINE is the evidence and the event is the nice-to-have, which is the opposite of how
this would have been built yesterday. D-018 established that PostHog answers 200 OK for events
it discards, so an exit recorded only in telemetry may leave no trace at all.
`record_daemon_stopped` is also the one capture here that is AWAITED rather than
fire-and-forget: every other event goes through `handle.spawn`, which is right for a running
daemon and useless on the exit path, where the spawned task is still queued when the process
goes away.

**REGRESSION INTRODUCED BY THIS FIX, found and fixed the same day.** The graceful path made
axum wait for every open connection after the signal, and a connection that never drains (one
stuck mid-request, a WebSocket) kept the process alive forever: it logged the line below, closed
its listener, and never exited. The check above passed only because that daemon had been up 3
seconds with nothing open. It went unnoticed for the rest of the day because `start-daemon.sh`
escalates an ignored SIGTERM to SIGKILL, so every restart LOOKED clean while the real sequence was
TERM, hang, KILL. When a restart bypassed that escalation (its output piped to `head -1`, which
killed the script by SIGPIPE before it could escalate), the daemon went down: listening on
nothing, holding its pid. Fixed by bounding the drain: after the signal, a watchdog exits the
process after `TRIUMVIRATE_SHUTDOWN_DRAIN_SECS` (default 10). Reproduced before fixing, on a
throwaway daemon on its own port and home: the pre-fix binary was still running 25s after SIGTERM
with one stuck connection; the fixed build exited at the 3s limit. Now a permanent guard,
`scripts/verify-shutdown.py` (`verify-live-agents.sh shutdown`), which runs the binary directly
and never escalates, because the escalation is what hid this.

**CHECK PASSED, live, 2026-09-19:** SIGTERM to the running daemon produced
`WARN daemon shutting down reason=SIGTERM uptime_seconds=3`, followed by
`INFO tv_daemon_stopped posted status=200 OK`. Per D-018 that 200 says nothing about
delivery, which is why the warning above it is the thing that closes this row.


### 2026-09-19: three rows were stale, the defects were already fixed
Checked during the ask_jury work, each against the CHECK its own row demanded. None of the
three had been re-run since the fix landed, which is the failure mode this file exists to
prevent: a list nobody trusts is a list nobody reads.

**D-010 (sight gate cannot see a grok shell read).** Check was "dispatch grok with
`required_sources=[<file>]` and instruct it to `cat` the file". Run live 2026-09-19: grok made
49 tool calls, read the named source, and the gate ACCEPTED the turn. The shell-read
classification (`codex::shell_read_kind`, applied by every adapter) is what fixed it, and
`a_grok_cat_of_the_named_source_passes` guards it offline.

**D-012 (a failed gemini request reports the fallback hop's error and hides its own).** Check
was "force agy to return empty and confirm the error names agy, not the fallback hop".
Observed live this session, in the strict_agent tests and in a real dispatch: the terminal
error is now the whole chain, oldest first, `agy attempt 1/1: agy capacity/quota error (exit
2): Error: RESOURCE_EXHAUSTED quota exceeded -> degraded codex: no result.text message`. agy's
own failure leads. Fixed by `failure_chain` in `agent_exec.rs`.

**D-013 (a child's exit code was the whole error; the quota message was thrown away).** Check
was "exhaust a quota, dispatch, read the 502: the child's own words are in it". Happened
unprompted on 2026-09-19 while dispatching a peer review: `codex connector failed: exited with
status exit status: 1; codex said: Selected model is at capacity. Please try a different
model.` The child's words are carried verbatim.


### 2026-09-19: the sight gate discarded codex reviews that had read the sources
Codex reads with a chain (`wc -l F && sed -n '1,240p' F`, or two files in one command) and
the gate rejected those turns as "never successfully opened", throwing away correct reviews.
Two of three jury seats were lost on the first live run with no disagreement between them.

THREE parsers carried the same blanket refusal of any `&&`, which is what closed the D-010
decoy attacks. The refusal is right about the attacks and wrong about a chain, where each
link is its own command. Fixed with `agent-adapter::codex::and_chain_segments` (splits on
`&&` only, honoring quotes) applied in `command_reads_file_contents`,
`command_reads_whole_file`, and plural `whole_file_read_operands` / `command_read_ranges`,
consumed at all five gate sites. `;` and `||` stay refused because both exit zero on a read
that failed, as does an unterminated quote, which is a parse error that runs nothing. Every
segment still goes through the unchanged strict parser.

The one that actually mattered was `command_reads_file_contents`: it runs inside
`shell_read_kind`, so a chained read stayed `ToolKind::Bash` and the coverage check, which
filters on `ToolKind::ReadFile`, dropped the call before the other parsers were consulted.
Fixing the two downstream parsers changed nothing live, and it took three failed live runs to
find because the test helper hardcoded `kind: ToolKind::ReadFile` and so skipped the step
under test. The helper now derives the kind through `shell_read_kind` as the adapters do.

Two assertions in `codex_03` / `codex_05` were changed deliberately: they required
`ls /repo && cat /repo/a.rs` to fail closed because "the reader may not be the part that
touched the named path". That concern is now carried structurally. Classification answers
"does this read a file" and operand binding answers "which file", and `codex_03b` asserts the
decoy directly (`ls /repo/a.rs && cat /repo/b.rs` binds `b.rs` only). Mutation DETECTION is
unaffected: it filters on `WriteFile`/`EditFile` kinds and has always been blind to codex
shell commands, where the read-only sandbox is what holds.

Diagnosis was blocked for two rounds by the gate's own receipt, which truncated the command
at 160 characters, below the length of a real one. It now reports the binding: the operand
each reader opened and the window it took. Writing that test exposed the PART receipt
reporting the command's FIRST window whatever file it covered, so a reader who took lines
1-50 of the source was told it had read "lines 1-240" of another file's window.

**CHECK PASSED, live, 2026-09-19:** a sight-gated `ask_jury` over the brief returned
`codex: {status: answered, answered_by_agent: codex, tool_calls_made: 2, verdict: ready}`,
3 of 3 seats answered, where the three previous runs on the same brief lost that seat.
Offline: 96 adapter, 570 lib and 278 binary tests, and reverting the classifier turns the
new tests red where they previously passed straight through the bug.


### 2026-09-03: sight gate rejected every source-gated codex review as "never opened"
The codex read classifier allowed `cat`/`head`/`nl`; codex-cli 0.145.0 reads files as
`sed -n 'N,Mp' FILE` windows and `nl -ba FILE | sed -n`, so 4 of 4 gated reviews that day
were rejected, fresh worker and reused alike. The report that surfaced it blamed a stale
worker. Fixed in `agent-adapter::codex::command_read_range` (the two shapes, nothing wider)
and `agent_exec::codex_ranged_reads_cover_source` (windows unioned against the file's real
line count). Rejections now carry a receipt: line count and windows seen, or every call that
named the source. Check passed live: a 763-line file, 7 tool calls, gate PASSED; a reused
thread that skipped lines 1-260 was rejected PART with the receipt showing exactly that.

### 2026-09-03: grok Fast turn cap (6) below the floor for a review that opens files
Three default-depth review dispatches hit max-turns at 6 with 19 to 32 tool calls and
returned a one-line preamble. Raised to 12, and `grok_depth: "fast"|"deep"` added to
`AskAgentRequest` so depth is a property of the request, not of the daemon's environment.
The panel seat stays forced Fast. Check passed live: a request with `grok_depth: deep` on a
Fast daemon spawned `--effort high --max-turns 30` (sampled from the child's argv).

### 2026-09-03: comment misstated the ask timeout as 180s
`agent_exec.rs` said the caller's `ask_agent` timeout fires at 180s. It is
`DEFAULT_DAEMON_ASK_TIMEOUT_SECS` (900); 180s is the generic connector default. Two
confidently wrong conclusions were drawn from it before the code was read. Comment now names
the constant.

### 2026-07-28 — ask_agent timeout misreported as a dead daemon
Error source chain discarded, unconditional restart advice, and autostart firing on timeout
(one call, two paid dispatches). Fixed in `daemon-http` and `mcp-tools`, 8 tests, negative
control confirmed. See `2026-07-28-timeout-misreported-as-dead-daemon.md`.

### 2026-07-29 — git hooks inert since 2026-05-10 (two causes, not one)
Symlinks in `.git/hooks/` repointed via `scripts/install-git-hooks.sh` on 2026-07-28, and
`core.hooksPath` unset on 2026-07-29. The first fix alone did nothing: with `core.hooksPath`
set, git never reads `.git/hooks/`. Proven fixed by pushing a throwaway branch and watching
`pre-push: ✓ check + clippy passed` appear on a real push, not by running the script by
hand. The absence of detection for this class remains open as D-009.

### 2026-07-28 — clippy red on main, blocking CI
Four errors in `mcp-bridge` (orphaned doc block, doc list continuation, duplicated
`#[allow]`, collapsible if) and one in `triumvirate` (field assignment outside initializer).
`scripts/pre-push-ci-checks.sh` now passes.

### 2026-07-28 — flaky test: `daemon_core::pid::read_pid_from_path_rejects_garbage`
`unique_test_root()` keyed the temp dir on a nanosecond timestamp, but macOS reports that
clock at microsecond granularity, so parallel tests collided on one `daemon.pid`. Added an
atomic counter. Three consecutive full workspace runs clean.

### 2026-05-26 — ABE red-team stub detection not blocking
See `2026-05-26-abe-red-team-stub-detection-not-blocking.md`.

---

**Last reviewed:** 2026-09-20. D-021 RESOLVED at 16:34 ET: all four seats fixed and verified live, codex included (gpt-5.6-sol, read from the thread's rollout turn_context). The 14:04 conclusion that codex "cannot report the served model" was wrong, generalised from two probes, and Mike rejected it; the wrong paragraph is kept in the row on purpose. D-021 grok half FIXED and verified live at 11:11 ET (grok-4.6-build), leaving only the codex live check, blocked until 14:03 ET. D-020 through D-024 added (five telemetry attribution defects, all measured live in PostHog before filing). D-020, D-022, D-023 and D-024 then FIXED and CHECK PASSED against live PostHog rows the same day, on a freshly installed binary and a restarted daemon. D-021 is PARTIALLY fixed: deepseek verified live, codex blocked on its own quota until 14:03 ET, grok genuinely still open with no model source. D-004 CLOSED, its check passed on a real failure encountered during that verification. D-005 unblocked but NOT worked. D-025 added (an intermittent test, cause not established). The blocker recorded on D-004 and D-005, "nothing lands in PostHog until 14:03", was WRONG: ingestion was healthy all morning and 14:03 is Codex's unrelated usage quota.

**Superseded line:** **Last reviewed:** 2026-09-19 (D-019 added and closed during wiki step two; before that, every row re-checked against its own CHECK during the ask_jury work: 3 closed as stale, 3 confirmed still open with fresh evidence, 1 added)
