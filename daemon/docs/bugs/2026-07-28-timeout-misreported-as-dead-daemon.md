# BUG REPORT — client timeout reported as a dead daemon

**Date observed:** 2026-07-28, ~21:31-21:42 EDT.
**Daemon version:** `triumvirate-daemon-v2 3.9.0`.
**Daemon PID at time of observation:** `21396`, up continuously since 2026-07-24 12:54:47.
**Status:** ROOT-CAUSED AND FIXED (see Fixes below). Instrumentation gaps it exposed remain OPEN — see `daemon/docs/bugs/OPEN.md`.
**Supersedes hypothesis #2 of:** `2026-05-25-daemon-session-ask-intermittent-failure.md`.

## Symptom

`ask_agent` failed with:

```
ask_agent requires triumvirate daemon; daemon request failed:
error sending request for url (http://127.0.0.1:8080/ask-agent)
```

Read as connection-refused. The daemon was assumed dead and PostHog was searched for its
death. PostHog held only `BatchLogProcessor.ExportError` lines, so the conclusion was that
the listener had died without logging.

## What was actually true

The daemon never died.

```
$ ps -o pid,lstart,etime -p 21396
  PID STARTED                       ELAPSED
21396 Fri Jul 24 12:54:47 2026  04-08:57:16
```

Last `daemon listener bound` in `~/.triumvirate/daemon.log`: `2026-07-24T16:54:47Z`. No
rebind, no restart. It was listening throughout, including while the failure was being
investigated.

The client gives up at 180s (`DEFAULT_DAEMON_ASK_TIMEOUT_SECS`). The daemon does not.

| agy dispatch started (UTC) | completed | duration |
|---|---|---|
| 01:31:04.07 | 01:38:08.75 | 424s |
| 01:34:04.37 | superseded | client timeout |
| 01:38:49.81 | 01:47:13.50 | 504s |

Row two begins 180.3 seconds after row one: the 180s ceiling plus the hardcoded 300ms
autostart sleep. It is not a manual retry. It is the client timing out and the autostart
path re-sending a request the daemon had already accepted.

## Three defects

**1. The error's source chain was discarded.** `reqwest` renders every `Kind::Request`
failure as the same sentence. Refused and timed-out are identical in `Display`; only the
`source` chain separates them, and `daemon-http/src/lib.rs` bailed with `{e}`:

```rust
anyhow::bail!("daemon request failed: {e}")   // Display only. Source dropped.
```

This is verbatim hypothesis #2 of the 2026-05-25 report, which called it "*the* first
patch." It was not applied. The cost of that two-month gap was this incident plus a
published claim that had to be retracted.

**2. The remediation was unconditional.** `inter_agent.rs` appended
`start it with: triumvirate daemon` to every failure, so a healthy daemon mid-dispatch
was reported as not running, with a fix prescribed for a process that was already up.

**3. Autostart fired on timeout.** `attempt_daemon_autostart_once()` ran on any send error.
On a timeout it spawned a second daemon (which loses the race to bind 8080 and dies) and
re-sent an accepted, paid request. One tool call, two full 180s attempts. Visible in
PostHog as a single `$mcp_tool_call` of 360308 ms.

## Fixes

All in `daemon/crates/daemon-http/src/lib.rs` and `daemon/crates/mcp-tools/src/inter_agent.rs`.

- `DaemonRequestFailure` (`Timeout` / `Unreachable` / `Other`) plus `DaemonRequestError`
  carrying the classification and the full `source` chain via `error_chain()`.
- Classification checks `is_connect()` before `is_timeout()`: a connect that timed out is
  honestly unreachable, and that is the only case where restarting is a sane response.
- `describe_ask_agent_failure()` picks the remediation from the classification. Timeout
  says the daemon is still working and names `TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS`.
  Unclassified failures prescribe nothing.
- Autostart gated on `Unreachable`.
- Removed a stray `\\` in the old literal that printed a backslash into the message.

Verified end to end against the installed binary, not just the test harness:

```
ask_agent failed: daemon unreachable at http://127.0.0.1:59999/ask-agent: error sending
request for url (...): client error (Connect): tcp connect error: Connection refused (os error 61)
start it with: triumvirate daemon

ask_agent failed: daemon did not respond within 2s to http://127.0.0.1:59998/ask-agent (the
request was accepted; the daemon may still be working on it): error sending request for url
(...): operation timed out
the daemon is running and may still be finishing this request; raise
TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS (currently 2s) if the model needs longer
```

8 tests added, all passing. Negative control confirmed: reverting the autostart gate fails
`classifies_live_failures_and_only_autostarts_when_unreachable` on
"a timeout must never trigger autostart".

## What PostHog had the whole time

Not nothing. Three `$ai_generation` rows with `$ai_is_error=true` and latency of exactly
**180.002s** each, plus two `$mcp_tool_call` `ask_agent` errors at 360308 ms and 180003 ms.
Three failures at an identical duration is a client ceiling and cannot be a model.

The data was there and correctly recorded. What was missing was a chart pointed at it.
Added as "Client-ceiling timeouts" on dashboard 1886865.

## The lesson worth keeping

A confident error message is indistinguishable from a correct one, and it is worse than
silence, because silence prompts an investigation and a specific-sounding diagnosis closes
one. The correct record was on local disk in `~/.triumvirate/daemon.log` the entire time.
What kept us out of it was not a missing log. It was a log that told a plausible story.
