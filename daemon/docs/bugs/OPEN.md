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
**Found:** 2026-07-28 · **Severity:** MEDIUM
**Evidence:** the three failed `$ai_generation` events of 2026-07-28 carry
`tv_outcome = "unreported"` and nothing else. `$ai_error` does not exist in this project's
taxonomy.
**Why it matters:** we can see that a generation failed and for how long, never why. The
180.002s duration is currently the only diagnostic, and it only works because a client
timeout happens to be a constant.
**Check:** a failed generation in PostHog carries a cause string.

### D-005 — Five instrumentation streams have gone silent, cause unknown
**Found:** 2026-07-28 · **Severity:** MEDIUM
**Evidence:** hours since last event as of 2026-07-28: `tv_review_verdict` 167,
`tv_fleet_spawn` 167, `tv_review_requested` 167, `tv_codex_dispatch` 165, `tv_maintenance` 122.
**Why it matters:** unknown whether those code paths stopped running or their emitters broke.
Two dashboard tiles chart `tv_codex_dispatch` and have been flat zero for six days, which
reads as "no defects."
**Check:** for each stream, either exercise the path and see the event land, or confirm the
path genuinely has not run.
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
**Evidence:** both git hooks were symlinks to `/Users/mikeboscia/...` (a username that does
not exist on this machine) from 2026-05-10 to 2026-07-28. `ls -la` showed hooks present. Git
stats a dangling symlink, finds nothing, and silently runs no hook. Clippy went red on main
and no local gate objected.
**Why it matters:** this is the same failure class as everything above, applied to our own
tooling: a control that reports presence and does nothing. Nothing currently detects it.
**Check:** a startup or CI step that executes each configured guard and fails if one is
missing, unreadable, or non-executable.

---

## Closed

### 2026-07-28 — ask_agent timeout misreported as a dead daemon
Error source chain discarded, unconditional restart advice, and autostart firing on timeout
(one call, two paid dispatches). Fixed in `daemon-http` and `mcp-tools`, 8 tests, negative
control confirmed. See `2026-07-28-timeout-misreported-as-dead-daemon.md`.

### 2026-07-28 — both git hooks dangling since 2026-05-10
Repointed via `scripts/install-git-hooks.sh`; both now execute and exit 0. The absence of
detection for this class remains open as D-009.

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

**Last reviewed:** 2026-07-28
