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

**Last reviewed:** 2026-09-19 (every row re-checked against its own CHECK during the ask_jury work: 3 closed as stale, 3 confirmed still open with fresh evidence, 1 added)
