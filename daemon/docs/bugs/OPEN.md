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

### D-001 — The daemon has no shutdown event
**Found:** 2026-07-28 · **Severity:** HIGH
**Evidence:** `tv_daemon_started` fired 14 times in 30 days. There is no corresponding stop
event. Two SIGTERMs sent on 2026-07-28 (22:08, 22:29 EDT) produced no record at all.
**Why it matters:** a daemon that ends is visible only later, by the absence of traffic. A
crash and a clean restart are the same evidence. This is the specific case the defect
dashboard was built to catch, and it catches nothing because nothing is emitted.
**Check:** kill the daemon, then find an event or log line naming the shutdown and its cause.
**Tile:** "Daemon restarts — births with no recorded deaths" (dashboard 1886865).

### D-002 — OTLP export failures ship with an empty body
**Found:** 2026-07-28 · **Severity:** HIGH
**Evidence:** `BatchLogProcessor.ExportError` rows in PostHog carry `body: ""`. The cause
(`dns error: failed to lookup address information`, `TimedOut`) exists in `fields.error` in
`~/.triumvirate/daemon.log` and does not survive into PostHog.
**Why it matters:** the one signal that tells you telemetry is broken arrives carrying no
information about how it is broken.
**Check:** trigger an export failure (block DNS to us.i.posthog.com), confirm the PostHog row
carries the cause string.

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

### D-010 - sight gate cannot see a grok shell read, and its own message recommends one
**Found:** 2026-09-12 · **Severity:** MEDIUM
**Status 2026-09-13:** fixed in the working tree and live on the daemon. `shell_read_kind`
(codex.rs) classifies a shell command that reads a file as ReadFile, applied in the grok, agy,
claude and gemini adapters, so the class is closed and not the grok case. The gate binds a shell
read to its operand (`whole_file_read_operand`): a pipe, redirect, comment, subshell, or second
operand does not count, and a `description` naming the source does not match. Live: a grok
`cat` of a named source passes; `head -5` is rejected as PART. Panel: antigravity and grok
reviewed, antigravity's three bypasses closed; codex seat pending quota. Close after that pass.
**Evidence:** three `ask_agent` grok dispatches with `required_sources` on a 199-line diff
were rejected. Attempt three did what the rejection text asked and ran `cat <path>` in one
shell command. `agent-adapter/src/grok.rs:104` maps every `run_terminal_command` to
`ToolKind::Bash`; `tool_call_touched_source` (`agent_exec.rs`) counts only `ToolKind::ReadFile`.
The codex adapter classifies `cat` and `sed -n` reads (`codex.rs:104`) and the gate unions
codex sed windows; grok has neither. Attempt two used `read_file` and was also rejected as
"never opened", cause not yet established (the grok unified log records no arguments).
**Why it matters:** the rejection reads as "the reviewer did not open the file" when the
reviewer opened it. A gate that blames the reader for the instrument's blind spot trains the
operator to route around the gate, which is the rubber stamp it was built to end.
**Workaround:** dispatch grok with `require_sight` only and no `required_sources`, which is
what the gate's own parser-mode message recommends. Used 2026-09-12 for the codex 0.154 review.
**Check:** dispatch grok with `required_sources=[<file>]` and instruct it to `cat` the file;
the turn must pass. Then instruct it to `head -5` the file; the turn must be rejected as PART.

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

### D-012 - a failed gemini request reports the fallback hop's error and hides its own
**Found:** 2026-09-13 · **Severity:** HIGH (misdiagnosis in the field)
**Status 2026-09-13:** fixed in the working tree and live on the daemon. `execute_ask_agent`
collects a `failure_chain` and formats it oldest first; the chain reaches the 502 body, the
FAILED lifecycle and outbox event, and the dead-drop `reason`. Codex's stdout JSON error
events and stderr tail are in the connector error. Antigravity's empty result carries
`status`, `permission_requests`, and `tool_calls`. The bridge's four 300-byte body cuts are a
2000-char excerpt. Verified live: a Codex quota failure now reads "codex said: You've hit your
usage limit ... try again at 7:03 PM" on all three surfaces. Panel: antigravity and grok
reviewed; codex's seat pending its own quota reset. Close after the codex pass.
**Evidence:** an `ask_agent {agent: "gemini"}` review from the masterFFL session at 18:03Z was
served by agy (log: "gemini dispatch served by agy backend"). agy completed in 15s and again in
12s with "stream-json result carried no response text". The daemon then walked the default
degraded route `gemini-cli,codex`; the codex hop hit the 180s connector timeout. The 502 the
caller received said only `codex connector timed out`, and the dead drop's `reason:` says the
same. The calling session concluded "Gemini alias is mis-routing to Codex" and rerouted its
work. No mis-routing occurred; the original failure was never surfaced.
Two gaps compound it: (1) the agy parser reads `result.status` (`agy_stream.rs:175`) but the
WARN at `no response text` does not log it, and `RequestUserInput` tool events are not logged,
so whether agy stopped on a permission prompt or an error is unrecoverable; (2) agy's
`--log-file` is deleted after the run. A text-only replay of the same prompt on 2026-09-13
returned SUCCESS, so it is not a content refusal; it fails when agy tries to act.
**Why it matters:** an error that names the wrong component sends the operator to fix the
wrong thing. Same class as the closed 2026-07-28 "timeout misreported as a dead daemon".
**Fix shape:** the terminal error must carry the chain: `agy: empty response (status=X)` then
`degraded codex: connector timed out`. Log `status` and any `RequestUserInput` event at WARN on
the empty-response path. Keep the agy log on failure.
**Check:** force agy to return empty (e.g. a prompt that needs a permission it cannot get) and
read the 502 body: it must name agy first and codex second.

### D-013 - a child's exit code was the whole error; the quota message was thrown away
**Found:** 2026-09-13 · **Severity:** HIGH
**Evidence:** Codex over its usage limit. Every dispatch: "codex connector failed: exited with
status 1". The reason was on codex's stdout as `{"type":"error","message":"You've hit your
usage limit ..."}`, parsed by `CodexExecParser`, never surfaced. Stderr was drained to
`tracing::debug`. The claude connector already included stderr in its message
(`agent_exec.rs`, "Include the stderr"); codex and the two Antigravity connectors did not.
**Status:** codex fixed with D-012 (`codex_error_tail`, drain awaited, 300 chars per line,
fixture test from the real binary). The two Antigravity connector bails
(`agent_exec.rs` "Antigravity connector failed: exited with status") still carry no output;
they are the dead gemini-cli path and are left for step 5 of the recovery plan.
**Check:** exhaust a quota or revoke auth, dispatch, read the 502: the child's own words are in it.

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

## Closed

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

**Last reviewed:** 2026-09-03
