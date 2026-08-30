# Peer review: Grok adapter implementation guide

**Date:** 2026-08-30 · **Reviewers:** Codex (engineering), Antigravity/Gemini (architecture) · **DeepSeek:** not consulted, owner's call
**Subject:** `../grok-integration-spec.md`
**Status:** review only. No code written. Nothing in the guide implemented.

**Context given to both reviewers** (verified in the tree before asking):

- `is_supported_agent_name` (`mcp-bridge/src/lib.rs:94`) allows `gemini|codex|deepseek|claude`. Four advertised lists
  disagree with it and with each other: `cli_ops.rs:203`, `cli_ops.rs:223` say `[gemini,codex]`;
  `inter_agent.rs:313` and `main.rs:2175` say `[gemini,codex,deepseek]`; `main.rs:4059` is a test pinning that wrong
  answer. `claude` is dispatchable and advertised nowhere.
- **`claude` is half-built.** Dispatch arms at `agent_exec.rs:1560` and `:2643`, runner
  `run_claude_cli_process_with_session` at `:2486`, `claude_command()` at `mcp-bridge/src/lib.rs:299`. **No `claude.rs`
  in `agent-adapter/`.** The runner pushes a bare `-p`, passes no `--output-format`, and hand-builds a
  `ParsedAgentResult` inline: no stream parser, no session resume, no token usage.
- Owner's framing: the unfinished work is *"making Claude a callable agent so Triumvirate could be driven by a
  different CLI"*, which inverts the current assumption that Claude orchestrates and the others are consulted.

---

## THE DISAGREEMENT (do not resolve by averaging)

**On whether v1 should build the bespoke `streaming-json` parser at all.**

- **Codex: yes, duplication is correct here.** "The event schemas are not actually shared. Grok has `thought`,
  `text`, `usage`, `end`, `modelUsage`, `max_turns_reached`; Codex/Gemini have their own names and semantics." Agrees
  with the guide's §16.4 ban on extracting a generic parser in the same PR. "Extract later only after Grok and Claude
  expose repeated, proven structure."
- **Gemini: no, deferring ACP is the wrong cut.** "ACP (JSON-RPC) should move *into* v1, and the custom
  `streaming-json` parser should be abandoned. The guide mandates building a fragile parser for what is essentially
  Grok's internal debug output. Adopting ACP is the exact thing that would prove the adapter works structurally as a
  peer in an orchestrator-agnostic world."

The two are answering different questions. Codex is asked "is duplication correct for *this* PR" and answers within
the guide's frame. Gemini rejects the frame: if the goal is orchestrator-agnostic symmetry, another stdout scraper is
debt, not progress. **The guide's constraint #4 ("Do not start with ACP") is the exact decision under dispute, and it
was asserted, not argued.**

---

## CONVERGENCE (both reached independently, treat as confirmed)

### 1. The `-s` / `-r` first-turn resolution is unsafe as written

Guide §3.2 resolves conflicting upstream docs with "first turn `-s <uuid>`, later turns `-r`, trust the parser."

**Codex, on the concrete failure:** "process starts, then installed `grok` rejects `--session-id/-s` before producing
`end.sessionId`. At that point ... if the implementation pre-stores the generated UUID as the session record, the
daemon may retain a Grok id that never existed." He notes the current Claude runner already has the analogous
weakness. His rule: **first-turn `-s` must be provisional until `end.sessionId` arrives; if the CLI rejects it, fail
the turn and do not persist.** And do not silently retry mid-flight "unless stderr proves it was pure CLI arg
validation before token spend."

**Gemini, on the assumption:** mixing `-s` on turn 1 and `-r` on turn 2 mapping cleanly onto "Grok's black-box
on-disk session state without cross-talk, history leaks, or corruption is completely unvalidated."

### 2. The status-list drift is structural, not neglect

- **Gemini names the abstraction:** a missing **AgentRegistry**. "Agent identities are currently hardcoded across
  decoupled files ... A centralized registry trait should dynamically enumerate supported adapters at runtime."
- **Codex arrives at the same place** from the test side, arguing the Grok PR "should introduce only reusable seams
  that Claude can consume next, especially supported-agent helper and runner result validation."

The guide's proposed `supported_agent_names()` helper is the minimum version of this.

### 3. Claude-as-callable matters, but they differ on sequencing

- **Codex: parallel slice.** Not a prerequisite, because Grok can route through its own builder/parser/runner without
  depending on Claude. But related: "adding Grok will duplicate the subprocess adapter gap unless Claude is handled as
  a sibling shape." The Grok PR "should not finish Claude, but it should avoid making Claude worse."
- **Gemini: finish Claude first.** "Adding Grok is horizontal sprawl; it gives you a 5th LLM that operates on the
  exact same paradigm as the existing 4. Finishing Claude-as-callable is a vertical capability unlock ... Pausing a
  half-finished paradigm shift to bolt on another legacy-style adapter is a strategic error."

---

## CODEX: the acceptance checklist holes (§13)

This was the question that mattered most, and the answer is that **several items can pass while the feature is
broken**:

| Checklist item | How it passes while broken |
|---|---|
| `/status` includes `grok` | Passes while the allowlist and session dispatch stay inconsistent. |
| Dispatch match has `grok` arm | Passes while `run_agent_process_with_session` lacks its arm, or vice versa. **"Claude already proves two dispatch layers can drift."** |
| Parser fixture yields text/session/tokens/tool | Passes while the runner never uses the parser, never forwards events, or falls back to batch parsing wrongly. |
| resume ≠ new-session flags mixed | Passes while first-turn `--session-id` rejection persists a dead id. |
| Forbidden args rejected | Passes for exact flags but **not for `--flag=value` forms** unless tests cover both. |
| **Retry schedule length for grok is 1** | **Passes in the test-local closure while real `execute_ask_agent` still uses the generic schedule. The existing DeepSeek test is a reconstruction, not the real scheduler** (`agent_exec.rs:2968`). |
| doctor does not spend tokens | Passes while doctor never probes `--session-id`/`--resume`, "the highest-risk contract in §3.1/§3.2." |
| Existing tests still pass | Weak: `main.rs:4059` currently pins the wrong supported list. |

**Two acceptance items Codex says are missing:**

1. Mock end-to-end `spawn_session` + first `ask_session` + second `ask_session`, verifying the **persisted parser
   `sessionId`** is what gets passed to `--resume`, not the Triumvirate session name and not the provisional id.
2. Nonzero exit after partial stdout must not become a successful answer unless policy explicitly allows partials.

> The retry-schedule finding is the same defect class the Pantheon review named "The Free Ride": a test that asserts a
> condition against machinery it reconstructed itself, rather than against the machinery that runs in production.

## CODEX: `max_turns_reached` is judgment in the wrong layer

Guide §3.3 says fail closed "unless `text` already has a usable answer." Codex: **"Parser is the wrong place for the
'usable answer' judgment."** The parser should classify facts (`termination: MaxTurnsReached`, `completed: false`,
`response_text_nonempty: true`); the policy belongs in the runner or a result validator, which makes it testable:

```
max_turns + no text                        => Err
max_turns + text + allow_partial=false     => Err, partial attached/logged
max_turns + text + allow_partial=true      => Ok, marked incomplete
```

"The spec's current checklist item `max_turns_reached → fail-closed` is too vague to catch this."

## GEMINI: remaining unvalidated assumptions

1. **Format stability.** The guide treats `streaming-json` as a stable API contract while relying on undocumented
   event types including `auto_compact_*`. "A dangerous assumption that will break on upstream CLI updates."
2. **Zero-retry policy.** Single-attempt "assumes all failures are deterministic model errors and ignores transient
   network or auth token flakes inherent to remote CLIs, ensuring the adapter will be brittle in practice." Note this
   cuts against the guide's REQ-GROK-013 and against Codex's acceptance of it.

## GEMINI: on entrenchment (question A)

"It entrenches the hub-and-spoke screen-scraping model ... If Triumvirate is meant to be driven by Grok or Codex as
the prime orchestrator, the communication must be a symmetric protocol. Adding another bespoke parser makes inverting
the relationship more expensive because you are compounding legacy technical debt instead of migrating to a uniform
agent-to-agent protocol."

---

## What this changes, if anything is acted on

Nothing here has been implemented. Recording the decisions the review forces, so they are made deliberately:

1. **Constraint #4 ("do not start with ACP") is now contested and needs an actual argument.** It is the load-bearing
   scope decision and the guide asserts it in one line.
2. **Sequencing Grok against Claude-as-callable is an owner decision**, and the two reviewers split on it.
3. **The `-s` provisional-id rule should go into the spec regardless of sequencing.** Both reviewers hit it, and the
   guide's current text would produce a persisted session id for a session that never existed.
4. **The retry-schedule acceptance item is currently unfalsifiable** and should be rewritten to assert against the
   real scheduler.
5. **`main.rs:4059` must change in whatever commit fixes the lists**, since it pins the wrong answer today.
