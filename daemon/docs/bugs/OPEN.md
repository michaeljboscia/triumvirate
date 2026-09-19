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

### D-002 — OTLP export failures ship with an empty body
**Found:** 2026-07-28 · **Severity:** HIGH
**Evidence:** `BatchLogProcessor.ExportError` rows in PostHog carry `body: ""`. The cause
(`dns error: failed to lookup address information`, `TimedOut`) exists in `fields.error` in
`~/.triumvirate/daemon.log` and does not survive into PostHog.
**Why it matters:** the one signal that tells you telemetry is broken arrives carrying no
information about how it is broken.
**Check:** trigger an export failure (block DNS to us.i.posthog.com), confirm the PostHog row
carries the cause string.
**RE-CHECKED 2026-09-19, STILL OPEN, and it is not rare:** today's log holds **4,266**
`BatchLogProcessor.ExportError` lines. The newest still carries `body: ""`, with the cause
(`url: ".../i/v1/logs", source: TimedOut`) only in `fields.error`, exactly as first recorded
on 2026-07-28. Whatever volume was needed to make this worth fixing, it has arrived.

### D-003 — No gap marker for windows when logs did not ship
**Found:** 2026-07-28 · **Severity:** HIGH · **Depends on:** D-002
**Evidence:** during the DNS-failure windows on 2026-07-28, logs generated locally never
reached PostHog. Nothing marks those windows as untrusted.
**Why it matters:** absence of a log currently proves nothing. A quiet hour and a broken hour
render identically, which makes every "nothing happened" conclusion unsound.
**Check:** after an export outage, the affected window is explicitly marked as untrusted
rather than simply empty.

### D-004 — Failed generations carry no error text
**Found:** 2026-07-28 · **Severity:** MEDIUM · **Partially resolved 2026-08-07**
**Evidence:** the three failed `$ai_generation` events of 2026-07-28 carry
`tv_outcome = "unreported"` and nothing else. `$ai_error` does not exist in this project's
taxonomy. Recurred 2026-08-06: three more DeepSeek dispatches (`180.001s`, `68.083s`,
`180.0s`; `model=unknown`, one primary attempt, metered) landed as `unreported`.

**RESOLVED (the outcome half, 2026-08-07):** those drops were the caller's client-side
`ask_agent` ceiling (180s) cancelling the daemon's `execute_ask_agent` future before any
classify() arm ran, so `CallTelemetry` emitted its `unreported` default. `CallTelemetry` now
arms on dispatch (`begin_dispatch`) and, on a drop with no recorded outcome while in-flight,
emits `tv_outcome = "cancelled"` (an error outcome, visible to outcome-based monitoring). The
`unreported` sentinel is retained for a genuinely unclassified *synchronous* exit. DeepSeek is
the prone path: its absolute SLA is 1800s, far past the 180s client ceiling.

**Still open (the cause-string half):** a `cancelled`/`failure` generation still carries no
provider cause string — `$ai_error` does not exist in the taxonomy.
**Check:** a failed generation in PostHog carries a cause string.

### D-005 — Instrumentation streams gone silent, cause unknown
**Found:** 2026-07-28 · **Severity:** LOW (was MEDIUM) · **Partially resolved 2026-08-02**
**Evidence:** hours since last event as of 2026-07-28: `tv_review_verdict` 167,
`tv_fleet_spawn` 167, `tv_review_requested` 167, `tv_codex_dispatch` 165, `tv_maintenance` 122.

**RESOLVED for `tv_codex_dispatch` (2026-08-02):** the emitter is healthy. Over 30 days there
were 4 `dispatch_codex` plus 2 `dispatch_codex_worktree` MCP calls, and exactly 6
`tv_codex_dispatch` events. 1:1, nothing dropped. The stream is quiet because the path has
not been invoked since 2026-07-22, not because it broke. Recent project work
(`deliverability-control-plane`, 2026-07-30) was research and design, not code: 40
`gemini-search`, 12 `gemini-check-research`, 10 `gemini-deep-research`, 0 dispatches.

**Still open:** `tv_review_verdict`, `tv_review_requested`, `tv_fleet_spawn`, `tv_maintenance`.
The same cross-check is available for these — compare event counts against the corresponding
`$mcp_tool_call` counts — but review and fleet calls are too sparse (1-2 in 30 days) for the
comparison to prove anything yet.
**Why it matters:** a stream at zero is ambiguous between "path idle" and "emitter broken",
and the two demand opposite responses.
**Check:** for each remaining stream, exercise the path once and confirm the event lands.
**Tile:** "Instrumentation freshness — dead signal or quiet one?" (dashboard 1886865).

### D-006 — agy health probe has never exercised its failure branch
**Found:** 2026-07-28 · **Severity:** MEDIUM
**Evidence:** 1783 `tv_agy_health` probes over 30 days, 100% `ok/ok/healthy`, zero failures.
**Why it matters:** a monitor that has never fired has been run, not tested. We do not know
that it can report unhealthy, and it is one of the few live signals we have.
**Check:** force the backend unhealthy and confirm the probe reports it.
**Tile:** "agy health probe — has its failure path ever run?" (dashboard 1886865).

### D-007 — agy is running past its version pin, warn-only
**Found:** 2026-07-28 · **Severity:** MEDIUM
**Evidence:** installed 1.1.8 against a pinned expected 1.1.5. Two daemons booted drifted on
2026-07-28. Drift proceeds unless `TRIUMVIRATE_AGY_STRICT_VERSION=true`.
**Why it matters:** every dispatch runs against an unvalidated binary.
**Check:** either validate 1.1.8 and move the pin, or set strict mode and pin down.
**Tile:** "agy version drift — what the pin says vs what is installed" (dashboard 1886865).

### D-008 — 2026-05-25 session/ask intermittent failure, hypotheses 1/3/4/5 unresolved
**Found:** 2026-05-25 · **Severity:** MEDIUM
**Evidence:** `2026-05-25-daemon-session-ask-intermittent-failure.md`. Hypothesis #2
(swallowed error cause) is fixed as of 2026-07-28. The Gemini-subprocess hang, session reuse
poisoning, worker-pool exhaustion, and multi-client race hypotheses were never tested.
**Why it matters:** unknown whether the original symptom still exists. It may have been
entirely hypothesis #2 misreading a timeout, which is now impossible.
**Check:** next occurrence will produce a classified error naming the real cause. Until one
occurs, this is untested rather than fixed.

### D-009 — No detection for a guard that is installed but inert
**Found:** 2026-07-28 · **Severity:** MEDIUM
**Evidence:** git hooks were dead on this machine from 2026-05-10 to 2026-07-29 in **two
independent ways**, and fixing the first did not fix the hooks:
1. Both symlinks in `.git/hooks/` pointed at `/Users/mikeboscia/...`, a username that does
   not exist here. `ls -la` showed hooks present; `head` on them said No such file or
   directory. Repointed 2026-07-28.
2. `core.hooksPath` in `.git/config` was ALSO set to `/Users/mikeboscia/projects/triumvirate/.git/hooks`.
   When that config is set, git uses it **exclusively** and never looks in `.git/hooks/`, so
   repointing the symlinks changed nothing. Unset 2026-07-29.
**Why it matters:** the same failure class as everything above, applied to our own tooling.
It also shows the verification trap: on 2026-07-28 the fix was "verified" by executing the
hook script by hand, which proves the script works and says nothing about whether git calls
it. Only a real `git push` distinguishes those.
**Check:** a startup or CI step that pushes a throwaway ref (or otherwise triggers each
guard through its real entry point) and fails if the guard produces no output. Verifying the
artifact is not verifying the path.

---

### D-011 - codex argv is assembled on four surfaces and only two have a binary oracle
**Found:** 2026-09-12 · **Severity:** MEDIUM
**Evidence:** codex 0.154.0 removed `--full-auto` from `exec`. Four places build codex argv:
`mcp-tools/src/abe.rs` (`build_worker_argv`, `build_worktree_worker_argv`),
`triumvirate/src/agent_exec.rs` (consult, the `should_use_full_auto` branch), and
`fleet/src/orchestrator.rs`. Three of the four emitted a flag the binary rejects (`--full-auto`,
`--ask-for-approval never`, `--message`), and every test stayed green because the tests assert
what Triumvirate builds, not what the installed binary parses. All three were fixed 2026-09-12.
Only the two ABE builders got a parse oracle (`abe_binary_oracle_tests`); the consult and
fleet argv are built inline inside spawn code and have no oracle.
**Why it matters:** the next removed flag goes red on two surfaces and ships on two. This is
the "fix lands on one surface" shape again, with a test that certifies half the class.
**Fix shape:** lift each inline codex argv into a pure builder, then one table-driven test that
runs every builder's output through `codex <args> --help` on the installed binary.
**Check:** temporarily push a bogus flag into the consult or fleet argv; `cargo test` must fail.

### D-014 - agy quota detectors over-match glog noise
**Found:** 2026-09-13 (Grok, review of recovery step 4) · **Severity:** LOW
**Evidence:** `classify_failure_message` matches any "429" (glog thread ids hit it,
`conversation_manager.go` 2026-08-20) and any "quota" (`doRefreshQuota`,
`retrieveUserQuotaSummary` health lines classify as capacity/quota). `quota_signal_in_line` is
the narrower detector and still shares the "429" substring. A false quota classification now
also triggers the step 4 backoff and feeds the breaker.
**Why it matters:** a benign log line can back off a healthy call by 60s and nudge the breaker.
**Fix shape:** anchor "429" to `code 429` / `HTTP 429` / `(429)` and "quota" to
`RESOURCE_EXHAUSTED` / `quota exceeded` / `capacity`; add the two glog lines as negative fixtures.
**Check:** the 2026-08-20 thread-id line and a `doRefreshQuota` line classify AuthOrExec.

### D-015 - fleet task ids collide across fleets in the same repo
**Found:** 2026-09-13 (audit) · **Severity:** MEDIUM
**Evidence:** `tasks.task_id` is the PRIMARY KEY of the ledger's tasks table and the
orchestrator names tasks `T-001`, `T-002`, ... per fleet (`orchestrator.rs`, `format!("T-{:03}")`).
The second `fleet_spawn` in the audit repo failed at once: `UNIQUE constraint failed:
tasks.task_id`, recorded in `fleets.failure_reason`. One fleet per repo, ever, unless the
ledger is wiped.
**Why it matters:** the second fleet in any project fails before it spawns anything.
**Fix shape:** key tasks on (fleet_id, task_id), or name tasks `<fleet_id>-T-001`. The merge
queue, branch names (`fleet/<fleet_id>/T-001`) and worktree names read the task id, so the
change has to land on all of them together; not a one-line fix.
**Check:** two consecutive `fleet_spawn` calls in one repo both reach `running`.

### D-016 - fleet_status cannot find a fleet after a daemon restart
**Found:** 2026-09-13 (Codex, review of the recovery commits) · **Severity:** MEDIUM
**Evidence:** `fleet_status` looks the fleet up in the daemon's in-memory map first and only
then refreshes from the ledger; the map is empty after a restart, and the ledger it would
read lives under a `project_root` that only the map knew. `FleetStatusRequest` carries only
`fleet_id`. The `fleets` table already has `source_project_root`, but the daemon does not know
which repo's ledger to open.
**Fix shape:** a daemon-level index `~/.triumvirate/fleets.json` mapping fleet_id to
project_root, written at spawn, read on a miss; or an optional `project_root` on the request.
**Check:** spawn a fleet, restart the daemon, `fleet_status` returns the ledger state.

### D-018 - PostHog answers 200 OK for events it discards, so delivery is unknowable from here
**Found:** 2026-09-19 · **Severity:** HIGH (was MEDIUM; raised once the cause was measured)
**Supersedes the first version of this row, which was WRONG.** I wrote it as "a quota rejection
is dropped without a single log line", which assumed a rejection existed and the daemon was
failing to log it. There is no rejection.

**Evidence, measured 2026-09-19 while the account was over quota:**
```
POST https://us.i.posthog.com/i/v0/e/   ->   HTTP 200   {"status":"Ok"}
SELECT ... FROM events WHERE event = 'tv_quota_probe' ...   ->   0 rows
```
The event was accepted, acknowledged as Ok, and discarded server side. `capture_as` in
`mcp-bridge/src/posthog.rs` handles this correctly for everything it can see: it warns on a
non-2xx and on a transport error, and logs the 2xx at `debug!`. There was nothing to warn
about. The daemon is not failing to report a failure; it is being told it succeeded.

**Why it matters:** the sending side cannot answer "did my telemetry arrive". A delivered
event and a discarded one are byte-identical from here. That is why three days of dead
telemetry looked like "nothing happened", and why the first suspicion fell on newly added
instrumentation rather than on the account.

**This collapses four rows into one problem.** D-002 (export failures ship an empty body),
D-003 (no gap marker for windows when logs did not ship) and D-005 (streams gone silent, cause
unknown) are all the same question asked from the same blind side. D-005 asked whether a quiet
stream means "path idle" or "emitter broken"; the answer needs evidence that does not come
from the emitter.

**Fix shape (the only one that actually answers it):** a delivery round trip. Emit a sentinel
event on a timer, then READ IT BACK through the PostHog query API (a `phx_` personal key,
separate from the ingest key). Delivery confirmed = the window is trustworthy. Sentinel not
returned within N intervals = mark the window UNTRUSTED and say so loudly, which is exactly
what D-003 asks for. Nothing short of a round trip distinguishes the two states, because the
ingest endpoint reports Ok for both.
**Check:** with the account over quota, the daemon reports telemetry as UNTRUSTED within one
sentinel interval, rather than reporting nothing.
**Consequence meanwhile:** `tv_jury_seat`, added with `ask_jury`, has never been observed
landing and cannot be until quota is restored. It is written and unverified.


## Closed

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
