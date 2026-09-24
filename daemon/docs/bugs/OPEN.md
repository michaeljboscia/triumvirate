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

### D-032 - The sight gate credits the read a reviewer ASKED for, not the one it received
**Found:** 2026-09-24 (D-028 panel, Codex) · **Severity:** LOW today, structural · **NOT FIXED**
**Evidence:** `structured_read_ranges` (`triumvirate/src/agent_exec.rs`) builds coverage from the `offset`/`limit` arguments of a successful read. `ToolCallRecord` carries arguments and a success flag, and nothing about the returned content, so a tool that caps or truncates its output is credited with the whole window it requested. The same record shape means an `offset` with no `limit` is credited to EOF.
**Second half:** offset conventions are assumed 1-based across every read tool. A zero-based tool asking `offset: 1` actually returns lines 2..N while the gate credits 1..N-1, so line 1 can be credited unread.
**Why it is filed rather than fixed:** closing it needs the adapters to record how many lines came BACK, which is a parser change per agent, or a per-tool semantics table. Both are larger than the defect. The shell path is unaffected: `sed -n` windows are exact.
**CHECK to close:** a read whose output was truncated does not satisfy a source; a zero-based reader's windows are credited at their true lines.

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
**Unblocks:** 2026-09-20 14:03 ET, when the quota resets.
**Check:** with `/health` reporting `telemetry_delivery: trusted`, exercise each remaining
stream once and find the event in PostHog.

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

## Closed

### D-028 - The sight gate rejects a COMPLETE read served as limit/offset windows
**Found:** 2026-09-22 (twice, on the D-027 panel's grok seat) · **Severity:** MEDIUM, and it discredits the guard · **CLOSED 2026-09-23**
**Evidence:** `read_args_are_partial()` (`triumvirate/src/agent_exec.rs:2376`) sends shell reads to `command_reads_whole_file()`, where a coverage UNION over `sed` windows runs (`agent-adapter/src/codex.rs:230`). Every other tool is marked partial if `limit`/`offset`/`start_line`/`end_line`/`line_offset`/`max_lines` is merely present. No union. Coverage is never computed.
From the gate's own rejection text: `agy_resilience.rs`, 750 lines, read as `[offset 1, limit 750]`, rejected. `orchestrator.rs`, 1844 lines, read as `[1..1000]` + `[1001..1844]`, no gap, rejected.
**Cost:** two complete grok deep reviews discarded, and the reviewer blamed for a read it performed correctly. The operator's natural response is to drop `require_sight`, which is the guard standing between a real review and one written from memory.
**Same shape as D-027:** two code paths for one job, and only one of them is right.
**CHECK to close:** a review whose only reads are adjacent limit/offset windows covering every line of a named source is ACCEPTED; one that leaves a gap is still rejected.
**Closed 2026-09-23.** `structured_read_ranges` feeds limit/offset windows into the SAME union the shell path uses, so tiled reads covering every line are accepted and a gap is still rejected. Related, and the reason this kept biting: a RELATIVE `required_sources` entry is now refused up front, naming the absolute path to pass, instead of running the review and rejecting it as unread afterwards; candidate expansion is also symmetric now. Structured ranges are bound to the read OPERAND (a known set of path-naming keys), not to the path appearing anywhere in the argument blob, which credited one file with another's coverage; and `limit: 0` contributes nothing, where it used to credit a line and could fabricate whole-file coverage across offsets (Codex, panel). Pins: `sight_39_structured_windows_that_tile_the_file_are_a_whole_read` (two controls: a gap still rejects, and full coverage of a DIFFERENT file proves nothing), `sight_40_a_relative_required_source_is_rejected_before_the_review_runs`, `sight_41_candidates_expand_in_both_directions`. Mutation-checked: removing the union turns sight_39 red.

### D-029 - complete_fleet approves the review gate itself, then merges
**Found:** 2026-09-22 (D-027 panel, Grok) · **Severity:** HIGH, and QUIET · **CLOSED 2026-09-23**
**Evidence:** a fleet worker queues a review with `dispatch_owned: false` (`fleet/src/orchestrator.rs:564`) and never dispatches a reviewer. `complete_fleet` then calls `set_review_status(task_id, ReviewGateState::Approved, None)` (`:923`) and `merge_next`, which blocks on `Pending` and proceeds on `Approved` (`fleet/src/merge.rs:82`).
**Result:** fleet state `done`, a `fleet_done` event, review gate `Approved`, comment `None`, and NO reviewer process ever ran. The gate that would have stopped the merge is stamped by the code that wants to merge.
**Why it is the D-027 shape:** a seat you believe is in the path did nothing, and the operation reported success. No env var, no quota failure needed.
**CHECK to close:** a task whose reviewer never ran cannot reach `Approved`; merge blocks or the fleet fails, and the ledger says which.
**Closed 2026-09-24, after the first fix was WRONG.** The first version read `ReviewRecord.state` for "approved"/"changes_requested". `submit_review` writes neither: it sets `state = 'done'` and puts the decision in `verdict` (approve|concerns|reject|indeterminate). A real REJECT therefore matched nothing and fell through to auto-approve, merging rejected code while labelling it "no reviewer ran". Codex and Grok both caught it; the test that "proved" the fix wrote `changes_requested` by hand, a state production never writes, so it validated an invented state machine. The gate now reads the verdict of the newest ANSWERED review (ties broken by rowid, because `datetime('now')` resolves to one second), fails CLOSED when the review store cannot be read, and lives in `review_gate_decision()` so that path is directly testable. `blocked_on_review` is admitted back into `complete_fleet` so an approval after a block can still merge; GC treats it as inactive and the CLI audit poller treats it as terminal instead of cancelling it. A `changes_requested`/`rejected` verdict parks the fleet in the new `blocked_on_review` state and it does not merge. An unreviewed task still merges by default (blocking there would strand every fleet on a step nothing drives today) but the row now SAYS it was auto-approved and a `review_auto_approved` event is written; `TRIUMVIRATE_FLEET_REQUIRE_REVIEW=1` blocks instead. Pins: `d029_a_rejected_review_blocks_the_merge` and `d029_an_unreviewed_task_still_merges_but_says_it_was_auto_approved`.

### D-030 - A review verdict cannot say who answered, so a substituted reviewer is accepted
**Found:** 2026-09-22 (D-027 panel, Codex and Grok independently) · **Severity:** HIGH · **CLOSED 2026-09-23**
**Evidence:** four surfaces drop or fake reviewer identity.
- `mcp-tools/src/gemini_query.rs:61` dispatches the gemini seat with `..Default::default()`, so `strict_agent` is off, then discards `answered_by_agent` and keyword-scans the text. `:76` defaults to `Clean`. With the route opted in to codex, a codex `Approved.` becomes `QueryGeminiReviewResponse { verdict: Clean }` with no field able to say codex answered. Reached from `main.rs:1040/1052/1064/1076`.
- Mandatory peer review builds its reviewer request with `..Default::default()` (`agent_exec.rs:2966`) and classifies from `resp.response` only (`:2981`). The `agy -> gemini-cli` hop has NO warning prefix (`:1611`), so a same-agent backend swap is invisible and its `APPROVE` is accepted.
- `peer-review/src/lib.rs:50` and `:339` compare reviewer names raw, but `normalize_agent_name` (`mcp-bridge/src/lib.rs:92`) maps `antigravity`/`agy` to `gemini`. `author_agent: "antigravity"` is assigned reviewer `gemini`: the author reviews itself. Same for `grok` vs `xai`/`supergrok`. `jury.rs:957` already canonicalizes; peer-review does not.
- `main.rs:1238/1248` hardcodes `author_agent: "codex"` in legacy `code_review`, so a gemini or grok author is recorded as codex AND stays eligible to review itself.
**Why it matters:** D-027 made substitution opt-in, but every one of these accepts a substituted or self-reviewing seat without being able to report it. `ask_jury` is the one surface that does this right (`jury.rs:344/445`): strict, plus provenance validation.
**CHECK to close:** a review response carries the answering agent and backend; a verdict from an agent other than the one asked is rejected, not scored; an alias cannot select the author as reviewer.
**Closed 2026-09-24.** `query_gemini_review` and the mandatory peer-review dispatch are now `strict_agent`, and both verify `answered_by_agent` before a verdict counts. Absent provenance is refused rather than assumed to be the assigned agent: only a matching `answered_by_agent`, or the daemon's `strict_agent_honored` acknowledgement, counts as evidence (Codex, panel). `normalize_agent_name` moved to `shared_types` so peer-review and dispatch share ONE alias list, and both the self-review check and reviewer selection compare canonical identities. `code_review` now requires `author_agent` instead of claiming `codex`. Pins: `d030_an_alias_of_the_author_is_never_chosen_as_its_reviewer` (with a control proving gemini is otherwise selectable), plus the alias mapping test asserting authorship survives.

### D-031 - A failed codex substitution is recorded as the gemini seat's failure
**Found:** 2026-09-23 (D-027 panel, Antigravity) · **Severity:** MEDIUM · **CLOSED 2026-09-23**
**Evidence:** `fleet/src/orchestrator.rs` degraded path. When the agy task fails and the opted-in codex relaunch ALSO fails, the final `task_failed` event writes `agent: launch_agent` and `error: format!("agent exited with status {:?}", status.code())`. `launch_agent` is bound before the degrade and `status` is the FIRST attempt's exit status, so codex's failure is invisible: the row says gemini failed, with gemini's exit code. The comment directly above it claims this field is what distinguishes "a degraded codex failure" from "the requested gemini failing".
**Second finding, same review:** the `JoinError` handler added for panicking workers lives in `spawn_fleet_members`' await loop. With `wait: true` that loop runs in the caller's task, so a dropped request (client cancel, connection drop) takes the loop with it while the `tokio::spawn`ed workers keep running detached. A worker that panics after that point is unobserved again, and its task stays `in_progress`. Pre-existing for every other exit too; the handler narrows the window, it does not close it.
**CHECK to close:** a failed degraded relaunch is recorded with the agent that actually failed and that agent's error; a worker panicking after its caller was cancelled still reaches a terminal task state.
**Closed 2026-09-24.** The degraded path now records the agent that failed LAST with that attempt's own error, keeping the first attempt's code as `first_attempt_error`. The panic window is closed by `WorkerTerminalGuard`, which lives INSIDE the worker task, so it runs on panic and on abort even when the caller's await loop is gone. The guard is ARMED/DISARMED: the first version ran on every exit, so the last successful worker of every fleet emitted `fleet_worker_abandoned`, and a stale guard could fail a legitimate retry of the same task (Antigravity and Codex, both seats). It also never drove the fleet terminal, so the scenario it existed for stayed broken; it now calls `finalize_if_all_tasks_terminal` and takes a busy timeout. Pin: `d031_the_guard_rescues_only_a_worker_that_recorded_nothing`, covering armed-rescue, disarmed-inert, and never-overwrite-a-success.

### D-027 - Codex silently answered in the Gemini seat whenever agy failed
**Found:** 2026-09-21 · **Severity:** HIGH (a "three peer" panel was sometimes two peers, one twice) · **CLOSED 2026-09-22**
**Cause:** three substitution points, on two surfaces.
1. Ask path: `degraded_route_env()` defaulted `TRIUMVIRATE_GEMINI_DEGRADED_ROUTE` to `codex` (commit `f908ccb`). Nothing set the env var, so every agy quota/auth/exec failure, and every call while the breaker was open, was answered by Codex.
2. Fleet, breaker open: launched `codex` in place of gemini, hardcoded, never read the env var.
3. Fleet, failed agy task: relaunched as `codex`, hardcoded.
**Fix:** one reader, `mcp_bridge::agy_resilience::degraded_route_env()`, default `fail`. Fleet gates both swaps on `degraded_route_allows_codex()`. Substitution now needs an explicit `TRIUMVIRATE_GEMINI_DEGRADED_ROUTE=codex`.
**Pins:** `agy::quota_backoff_and_route_default_tests::default_degraded_route_substitutes_nobody`, `strict_agent_tests::strict_00_default_route_never_substitutes_codex` (ignored, run by `scripts/verify-live-agents.sh strict`), `orchestrator::tests::failed_gemini_task_is_not_relaunched_as_codex_by_default`. Each has a negative control that opts in and still sees codex. Putting the `codex` default back turned all three red (mutation run, 2026-09-21).
**Not covered at first:** the fleet breaker-open path had no test. The panel required one; it now exists (`breaker_open_gemini_task_launches_nobody_and_still_finishes_the_fleet`, plus its opt-in control), `#[ignore]`d because it opens the process-global breaker, and run from `scripts/verify-live-agents.sh strict`.
**CHECK to close:** binary installed via `scripts/install.sh` (done 2026-09-21) AND the daemon restarted on it, then `bash scripts/verify-live-agents.sh strict` passes.
**Closed 2026-09-22.** Every CHECK ran: binary rebuilt from this branch and installed 14:20:36; daemon 7612 (running since 2026-09-20, still substituting) replaced by pid 94278 at 14:20:56; `scripts/verify-live-agents.sh strict` passed.
**Live evidence, not just tests.** A gemini `ask_agent` at 14:30:49 hit the real agy quota (`RESOURCE_EXHAUSTED (code 429): Individual quota reached`). The call FAILED with agy's own error, lifecycle `... RETRY, FAILED, FALLBACK`, and `ps` showed no codex child spawned by the daemon. `FALLBACK` is the dead-drop record at `~/.triumvirate/dead-drop/ccd65e4f-...-gemini.md`, which names `agent: gemini` and the quota reason. Before this fix that same 429 returned a codex answer marked success.

**Last reviewed:** 2026-09-24 (D-028 through D-031 closed; their own panel found the D-029 fix reading the wrong column and the D-031 guard firing on success, both corrected here, and added D-032) (D-019 added and closed during wiki step two; before that, every row re-checked against its own CHECK during the ask_jury work: 3 closed as stale, 3 confirmed still open with fresh evidence, 1 added)
