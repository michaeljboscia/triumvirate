# Grok integration: test plan and implementation cycle

**Goal:** wire up Grok and modify Triumvirate configuration so Grok works the same way Codex does.
**Spec:** `grok-integration-spec.md` · **Evidence:** `findings/grok-*.md` · **Fixtures:** captured from a live binary.
**Status:** Slice A landed (342 tests green). B through F below.

---

## 0. The rule this plan obeys

**A test that cannot fail is not a gate.** This repo already contains one: `agent_exec.rs:2977` defines its own
`schedule_len_for` closure and asserts against that closure, so it would pass no matter what `execute_ask_agent`
does. Its comment admits it is "a minimal reconstruction." Every test below must be able to fail for the reason it
claims to guard, and the ones that replace reconstructions say so explicitly.

**Corollary:** the Slice A test `u_grok_05_advertised_list_and_allowlist_cannot_drift` is the template. It asserts an
invariant between two real things rather than restating a literal.

---

## 1. Test pyramid

| Level | Where | Network | What it proves |
|---|---|---|---|
| **Unit** | `mcp-bridge/src/grok.rs`, `agent-adapter/src/grok.rs` | none | Arg construction and parsing are correct in isolation |
| **Integration** | `triumvirate/tests/integration_{http,mcp}.rs` + mock binary | none | Dispatch, session, `/status`, telemetry wire together |
| **End to end** | `#[ignore]`, `TRIUMVIRATE_LIVE_GROK=1` | real | The whole path works against the actual CLI and subscription |

`cargo test` must pass with **no network and no `XAI_API_KEY`**. E2E is opt-in only.

---

## 2. Unit tests

### 2.1 Invocation builder, `mcp-bridge/src/grok.rs` (Slice B)

| ID | Test | Must fail when |
|---|---|---|
| U-B-01 | default consult emits `--output-format streaming-json`, `-p` last | format flag dropped, or `-p` not adjacent to prompt |
| U-B-02 | never emits `-c` / `--continue` | REQ-GROK-006 violated |
| U-B-03 | resume emits `--resume <id>` and **not** `--session-id` | both emitted; the CLI rejects that pairing without `--fork-session` |
| U-B-04 | new session emits `--session-id <uuid>` and not `--resume` | mixed |
| U-B-05 | **`resume=true` with empty/None id is an `Err`, never a bare `-r`** | builder falls through. **Bare `--resume` means "most recent in cwd", the exact cross-talk the spec bans for `--continue`.** Not in the guide; found by probing |
| U-B-06 | forbidden extra flag rejected, **both `--flag value` and `--flag=value` forms** | only the spaced form is checked |
| U-B-07 | forbidden list includes `--permission-mode`, `--json-schema`, `--tools`, `--disallowed-tools` | Triumvirate loses control of approval policy or context size |
| U-B-08 | yolo off by default; `TRIUMVIRATE_GROK_YOLO=1` injects `--always-approve` | consult silently auto-approves |
| U-B-09 | `--no-auto-update` and `--no-alt-screen` always present | REQ-GROK-010 |
| U-B-10 | streaming off emits `json` | fallback path unreachable |
| U-B-11 | model/effort/max-turns only when configured | operator config ignored or invented |
| U-B-12 | `--cwd` present when cwd non-empty | session lands in the wrong directory |
| U-B-13 | **isolated `HOME` + explicit `GROK_HOME` in the spawn env by default** | the consult inherits `~/.claude.json` and runs at roughly 67K context instead of 14K |

### 2.2 Parser, `agent-adapter/src/grok.rs` (Slice C)

Fixtures are **real captures**, already committed:
`tests/fixtures/grok-streaming-20260830.jsonl` (420 tools) and `grok-streaming-isolated-20260830.jsonl` (26 tools).

| ID | Test | Must fail when |
|---|---|---|
| U-C-01 | happy path yields `response_text == "pong"`, session id, tokens | any field mismapped |
| U-C-02 | **`thought` never reaches `response_text`** | CoT leaks to the operator. The real fixture carries 33 `thought` events for a one-word answer, so this is not hypothetical |
| U-C-03 | `end.sessionId` becomes `session_id` | REQ-GROK-007 |
| U-C-04 | token map per spec 3.4 incl. `reasoning_tokens` to `thinking_tokens` | mismap |
| U-C-05 | **`cache_read_input_tokens` captured separately, not folded into input** | cost math silently wrong. Verified: these are separate counters and conflating them produced a wrong measurement during investigation |
| U-C-06 | `total_cost_usd` and `modelUsage[].costUSD` captured | quota burn invisible. Moved to v1 |
| U-C-07 | non-JSON line ignored, parser still finishes | a banner line kills the turn |
| U-C-08 | unknown `type` returns None, does not fail | upstream adds an event and every turn breaks |
| U-C-09 | `error` maps to `WorkingState::Error` | failure reported as success |
| U-C-10 | `max_turns_reached` classified as fact, **policy decided in the runner** | judgment buried in the parser where it cannot be tested |
| U-C-11 | `tool_call` without `toolName` falls back to `title` then `"unknown"` | panic or empty tool name |
| U-C-12 | `parser_mode == "grok-streaming-json"` | REQ-GROK-018 |
| U-C-13 | batch `json` parser reads `text`/`sessionId`/`usage` | streaming-off path silently empty |

> **Gap, stated rather than hidden:** the committed fixtures contain **no `tool_call` or `tool_call_update` events**,
> because the probe prompt invoked no tools. U-C-11 and the tool-mapping rows in spec 3.5 are therefore written
> against the spec, not against observed bytes. **Capturing a tool-using fixture is a prerequisite for calling Slice C
> done.**

---

## 3. Integration tests, mock binary, no network

`tests/fixtures/mock_grok.sh` replays a committed fixture and honors `-p`, `--resume`, `--session-id`, exiting per a
scripted code. Wired via `TRIUMVIRATE_GROK_BIN`.

| ID | Test | Must fail when |
|---|---|---|
| I-01 | `/status` `supported_agents` contains `grok` | Slice A regressed |
| I-02 | `/status` renders `supported_agent_names()` exactly | a literal list reappears |
| I-03 | `ask_agent` with `agent: "supergrok"` is accepted and normalized | alias not applied at the boundary |
| I-04 | dispatch reaches `run_grok_cli_process_with_session` | wired to the generic path by mistake |
| I-05 | **both dispatch matches have a `grok` arm** | one drifts. **Claude already proves two dispatch layers can diverge** |
| I-06 | unknown agent still bails | allowlist bypassed |
| I-07 | **`spawn_session` then two `ask_session` calls: turn 2 passes turn 1's parsed `end.sessionId` to `--resume`** | the Triumvirate session *name* or the provisional uuid is passed instead. `--resume` accepts a **title**, so a name resolves silently to the wrong conversation |
| I-08 | parser `sessionId` differing from the requested one wins, with a warning | REQ-GROK-007 inverted |
| I-09 | one Grok on-disk session never shared across two Triumvirate session names | cross-talk |
| I-10 | **retry schedule for grok is 1, asserted against the real scheduler** | replaced reconstruction. This test explicitly supersedes the tautological `schedule_len_for` closure |
| I-11 | nonzero exit with partial stdout is **not** a success | partial answer reported as complete |
| I-12 | auth failure classified distinctly from other exit-1 | operator sent to the wrong fix |
| I-13 | `CallTelemetry` emits `agent="grok"` with tokens and cost | PostHog blind to Grok spend |
| I-14 | timeout kills the process group | orphaned 133MB process |
| I-15 | forbidden `TRIUMVIRATE_GROK_ARGS` rejected before spawn | operator overrides a Triumvirate-owned flag |

## 4. End to end, `#[ignore]`, `TRIUMVIRATE_LIVE_GROK=1`

| ID | Test | Proves |
|---|---|---|
| E-01 | live consult returns `pong` | the whole path works |
| E-02 | second `ask_session` recalls turn 1 | resume is real, not just flag-shaped |
| E-03 | **subscription auth only, `env -u XAI_API_KEY`** | no API key required. Already verified manually |
| E-04 | **isolated HOME keeps context near 14K, not 67K** | the cost control actually holds under the daemon |
| E-05 | a tool-using prompt produces `tool_call` + `tool_call_update` | closes the fixture gap above |
| E-06 | doctor reports binary, version, auth **kind**, and spends no tokens | REQ-GROK-016 |

---

## 5. Configuration parity with Codex

"Works the same way Codex does" spans surfaces beyond dispatch. Codex appears in 30 files, Antigravity in 9.

**In scope now (Codex parity for the consult path):**

| Surface | File | Why |
|---|---|---|
| Alias mapping | `mcp-tools/src/aliases.rs:166` | `map_target_to_agent` handles only gemini and codex, so `spawn_daemon target:"grok"` fails today. **DeepSeek and Claude are already broken here.** |
| Doctor | `cli_ops.rs` | binary, version, auth kind, no token spend |
| README | `daemon/README.md` | the `TRIUMVIRATE_GROK_*` matrix |
| Telemetry | free via `CallTelemetry` | but the call-site comment claiming subscription calls "cost exactly $0" needs correcting: true for dollars, false for quota |

**Deferred, and named so it is a decision rather than an omission:** peer-review panel membership
(`peer-review/src/lib.rs:42` is `["codex","gemini","claude"]`), fleet worktree swarms, ABE worker type,
token-economics scanner.

---

## 6. Cycle

| Slice | Deliverable | Gate |
|---|---|---|
| **A** | identity + `supported_agent_names()` | **DONE.** 342 tests green |
| **B** | invocation builder | U-B-01..13 |
| **C** | parser | U-C-01..13, plus a tool-using fixture |
| **D** | spawn/dispatch + isolated HOME + cost capture | I-01..15 |
| **E** | doctor, README, `aliases.rs` | I-12, E-06 |
| **F** | mock binary, E2E suite | E-01..06 |

**Definition of done:** `cargo test` green with no network and no API key; `/status` lists grok; a live
`spawn_session` plus two `ask_session` calls resume correctly; PostHog shows a grok generation with tokens and cost;
and every deferred item above is written down as deferred rather than silently absent.
