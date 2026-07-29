# BUG REPORT — triumvirate daemon: `/session/ask` intermittent failure

**STATUS: PARTIALLY RESOLVED 2026-07-28** — hypothesis #2 below ("error-wrapping swallows
the cause", called out here as "*the* first patch") was correct and went unapplied for two
months. It is now fixed: `daemon-http` classifies the failure and preserves the full source
chain, so `error sending request for url (...)` can no longer hide a timeout, a refusal, and
a decode error behind one sentence. See
`2026-07-28-timeout-misreported-as-dead-daemon.md`, where the same string caused a healthy
daemon to be declared dead.

Hypotheses #1, #3, #4, #5 were never tested and remain open as D-008 in `OPEN.md`. It is
possible the original symptom was entirely a timeout misread as a transport failure, which
the fix makes impossible to repeat. Treat that as untested, not confirmed: the next
occurrence will name its own cause.

**Date observed:** 2026-05-25, throughout a multi-hour `/goatrodeo` ceremony.
**Daemon version:** `triumvirate-daemon-v2 3.9.0` (per `daemon_health`).
**Bind addr:** `127.0.0.1:18180`.
**Daemon PID at time of observation:** `21993` (see `~/.triumvirate/daemon.pid`).
**Mode:** `incremental-dev` (per `daemon_health.mode`).

## Symptom
The daemon's HTTP `POST /session/ask` endpoint returns failures intermittently — leaving
MCP `ask_daemon` and `ask_agent` both broken — while every other endpoint we hit (`/ping`,
`spawn_daemon`/`spawn_session`, `list_daemons`/`list_sessions`, `daemon_health`) continues to
work normally. The failures bias **heavily toward Gemini sessions**; Codex sessions kept
working most of the time.

## Exact error strings observed (verbatim)
Two distinct lower-level errors surfaced, wrapped by the same MCP-side wrapper at
`daemon/crates/mcp-tools/src/inter_agent.rs:144`
(`format!("ask_session via daemon failed: {e}")`):

1. Early in the session:
   ```
   ask_session via daemon failed: error sending request for url (http://127.0.0.1:18180/session/ask)
   ```
   (Network/transport-level — the daemon either didn't accept the connection or dropped it
   before responding.)

2. Later in the session (same daemon PID 21993 the whole time):
   ```
   ask_session via daemon failed: daemon request failed
   ```
   (Application-level — the daemon RESPONDED, but with a generic failure. This is the more
   common pattern; the underlying error message is being lost/swallowed somewhere between the
   daemon HTTP response and the MCP-side wrapper.)

3. Also the one-shot path failed identically (after the session path was failing):
   ```
   ask_agent requires triumvirate daemon; daemon request failed: daemon request failed
   ```
   This indicates **`ask_agent` and `ask_daemon` share the same broken dispatch path** — they
   both go through `session/ask` under the hood.

## Endpoints — what worked vs what failed (during the same session)
| Endpoint | Status across the session |
|---|---|
| `mcp__triumvirate__ping` | ✅ always returned `pong` |
| `mcp__triumvirate__daemon_health` | ✅ always returned `{status:"ok", version:"3.9.0", auth:null, daemon:null, …}` |
| `mcp__triumvirate__list_daemons` (= `list_sessions`) | ✅ always returned the session list |
| `mcp__triumvirate__spawn_daemon` (= `spawn_session`) | ✅ always returned a session record (often "reused for X") |
| `mcp__triumvirate__ask_daemon` (= `ask_session`) | ⚠️ **INTERMITTENT** — worked for Codex sessions most of the time; failed for Gemini sessions repeatedly; sometimes failed for Codex |
| `mcp__triumvirate__ask_agent` (one-shot) | ⚠️ FAILED with the same dispatch error once the session path started failing |

## Pattern observations
- **Codex daemons (`deepseek-rodeo-codex`, `deepseek-delta1/2/3-codex`, `deepseek-edit-codex`,
  `deepseek-p44-codex`, `deepseek-p44b-codex`) almost always worked.** The same calls against
  Gemini daemons (`deepseek-rodeo-gemini`, `deepseek-delta1-gemini`, `deepseek-p44-gemini`,
  `deepseek-p44b-gemini`) failed disproportionately. → strongly suggests the issue is in the
  **gemini-backend code path** (subprocess launching, OAuth refresh, agy-vs-gemini-cli switch),
  not the HTTP transport.
- `spawn_session` always reported "reused for X" even for genuinely new names → the daemon
  appears to key sessions by `(agent, cwd)` and return whichever existing record matches, not
  by the name. This may not be a bug, but it's a UX gotcha — and reused-but-tainted sessions
  could be relevant here.
- The MCP-side wrapper at `inter_agent.rs:144` is **swallowing the underlying error** into the
  generic `"daemon request failed"` string. The actual error from the daemon HTTP response is
  being lost. **First high-leverage fix:** make the wrapper log/propagate the daemon's HTTP
  response body or status code instead of stripping it.

## Code pointers (the call graph for `/session/ask`)
**MCP side** (Claude → MCP → daemon HTTP):
- `daemon/crates/mcp-tools/src/inter_agent.rs:134` — `pub async fn ask_session(...)`.
- `daemon/crates/mcp-tools/src/inter_agent.rs:144` — the wrapping that loses the error detail:
  `.map_err(|e| format!("ask_session via daemon failed: {e}"))`.
- `daemon/crates/mcp-tools/src/aliases.rs:200` — `map_ask_daemon_params` (the legacy alias).

**Daemon side** (HTTP route + handler):
- `daemon/crates/triumvirate/src/main.rs:2391` — `.route("/session/ask", post(session_ask_route))`
- `daemon/crates/triumvirate/src/main.rs:2036` — `async fn session_ask_route(...)`
- `daemon/crates/triumvirate/src/main.rs:501` — `async fn ask_session(...)` (the internal one
  the route dispatches to)

## On-disk state to inspect
- **`~/.triumvirate/sessions.json`** (238KB) — full session state for all spawned sessions
  during the ceremony; contains `deepseek-rodeo-gemini`, `deepseek-p44-gemini`,
  `deepseek-p44b-gemini`, `deepseek-delta1-gemini`, etc. Diff their structure against the
  working Codex sessions (same file) — anything that's `null`/empty/stuck for Gemini but
  populated for Codex points at the gemini path.
- **`~/.triumvirate/workers.json`** (2.7KB) — worker state at the time of the last write.
- **`~/.triumvirate/outbox.jsonl`** (1.3MB) — outbox event stream; look for events near the
  failure timestamps (around 9-10 PM EDT 2026-05-25) for the daemon-side trace.
- **`~/.triumvirate/dead-drop/`** — 3 gemini message files from May 22-23 (pre-existing); the
  mechanism for "messages that couldn't be delivered" already exists and may have been used
  during this session — check if new files appeared today.
- **`~/.triumvirate/daemon.pid`** = 21993 — the running HTTP daemon process for the whole
  ceremony.

## Reproduction recipe (rough)
1. Run a long `/goatrodeo` ceremony with multiple rounds of `ask_daemon` against both Codex
   and Gemini daemons (the failures correlate with sustained Gemini usage).
2. After ~2-3 successful Gemini calls, the next `ask_daemon` against ANY Gemini session
   begins failing with `"daemon request failed"`. Codex sessions usually keep working.
3. Spawning a new Gemini session does NOT fix it (the daemon reuses the existing record).
4. `ping` / `list_daemons` / `daemon_health` continue returning `ok` throughout.

## Hypotheses to investigate (ranked)
1. **Gemini-subprocess hang/leak.** The agy/gemini-cli subprocess launched by the daemon
   inside `ask_session` may hang, leak, or fail in a way that the daemon catches but reports
   generically. Check `daemon/crates/mcp-bridge/src/agy.rs` + `agy_resilience.rs` for failure
   paths that drop the underlying error message. Especially check OAuth token refresh /
   keyring access paths — possible blocking call on macOS Keychain.
2. **Error-wrapping swallows the cause.** `inter_agent.rs:144` returns just the `Display`
   string; if the daemon returns a JSON body with detail, it never reaches the caller.
   Trivial fix: propagate the daemon's HTTP status code + response body. This is *the*
   first patch — it makes every subsequent hypothesis cheaper to test.
3. **Spawn-by-(agent,cwd) reuse poisoning.** If the daemon keys sessions by `(agent, cwd)` and
   reuses, a single broken Gemini session can poison every subsequent spawn with the same
   key. Spawning under a different name returns the SAME broken session. Verify the keying in
   the spawn handler and confirm whether a "fresh" name actually gets a fresh subprocess.
4. **Worker pool exhaustion / semaphore deadlock.** Long ceremonies + many spawns may have
   leaked permits or stuck a worker. `workers.json` should show this.
5. **Multiple `triumvirate mcp` client processes** (5 were observed via `ps` — PIDs 95513,
   70539, 64008, 63673, 21993). Only 21993 holds the listen port; the rest are stdio MCP
   client subprocesses (one per Claude Code session). Probably fine, but if any of them
   stale-locked a shared resource (`sessions.json` write lock?), this could surface as a
   race. Verify `sessions.json` write atomicity.

## Quickest things to try
1. **Patch `inter_agent.rs:144`** to log the daemon's actual HTTP response body/status. Even
   just `eprintln!`-ing it locally will reveal what the daemon is actually saying.
2. **Restart the daemon** (kill PID 21993, restart) — if Gemini sessions then work fine for
   a while and re-degrade, it's a memory/handle leak in the Gemini subprocess path.
3. **Strace/dtruss the daemon** while running an `ask_daemon` against a Gemini session — see
   what the subprocess is actually doing.
4. **Reproduce with Codex-only**: spawn 10 Codex daemons + 100 asks. If it stays green,
   confirms the issue is Gemini-specific.

## Related context from this ceremony (for the record)
- The ceremony successfully completed: 3 spec-review rounds + verification + EDIT + Phase 3 +
  Phase 4. Documented at `daemon/docs/specs/deepseek-integration-spec.md` + `daemon/docs/v1-deepseek/`.
- Despite the daemon issue, every required twin audit completed — Codex always reached us
  (daemon path), Gemini via the `gemini-query` MCP fallback (which doesn't have repo file
  access — a degradation but workable).
- Goatrodeo events log (with timestamps for each gate): `.goatrodeo.events.log` (67 lines as
  of bug-write).
