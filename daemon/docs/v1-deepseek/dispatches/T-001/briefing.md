# Task T-001 — Make `deepseek` a recognized agent surface

> **REQ-IDs:** REQ-DS-001, REQ-DS-013, REQ-DS-016, REQ-DS-022.
> **Wave:** 1 · **Depends on:** none · **Base SHA:** `841f685` (= `spec/deepseek-integration-v1`).
> **Spec source of truth:** `daemon/docs/specs/deepseek-integration-spec.md` §3.1.
> **You are a build worker.** Read the rules, execute, commit, write the sentinel, notify the daemon. Do not improvise scope.

## What this task IS

A pure **surface-wiring** change: make the daemon accept `"deepseek"` as a valid agent name
everywhere the gate is currently hardcoded to `gemini` / `codex`. NO new behavior, NO
dispatch routing, NO HTTP runner — just admit a new agent name to the surface.

Without this task, every `ask_agent {agent:"deepseek"}` returns "unsupported agent" and the
rest of the build can't be exercised. After this task, requests reach the routing layer
(which will error later with "no deepseek backend" until T-012 lands — that's expected).

## What to change (exact surface points)

| File | Around | Change |
|------|--------|--------|
| `daemon/crates/mcp-bridge/src/lib.rs` | `is_supported_agent_name`, line ~37-39 | Extend to accept `"deepseek"`. Today: `agent == "gemini" \|\| agent == "codex"`. |
| `daemon/crates/triumvirate/src/main.rs` | `/status` `supported_agents` array, ~line 1874 | Add `"deepseek"`. |
| `daemon/crates/triumvirate/src/main.rs` | session-spawn unknown-agent error text, ~line 2008 | Include `"deepseek"` in the known-agents list of the message. |
| `daemon/crates/triumvirate/src/agent_exec.rs` | `execute_ask_agent` unsupported-agent error text, ~line 247 | Include `"deepseek"` in the known-agents list of the message. |
| `daemon/crates/mcp-tools/src/lib.rs` | `display_agent_name`, line ~92 | `display_agent_name("deepseek")` returns `"DeepSeek"`. (The function lives at the crate root, NOT in inter_agent.rs.) |
| `daemon/crates/mcp-tools/src/inter_agent.rs` | supported-agents fallback list, line ~275 | The hardcoded `vec!["gemini", "codex"]` fallback gains `"deepseek"`. |

Line numbers are approximate (read the real code; don't blindly trust). The contract's
`allowed_files` enumerates the only files you may modify.

## What NOT to change (HARD)

- DO NOT add a DeepSeek HTTP runner — that's T-010.
- DO NOT add dispatch routing for `deepseek` (i.e., a `"deepseek"` arm in
  `run_named_agent_with_session_and_model`) — that's T-012/T-014.
- DO NOT modify Gemini or Codex behavior — their existing tests MUST stay green.
- DO NOT touch `daemon/crates/mcp-bridge/src/agy.rs` or `agy_resilience.rs`.
- DO NOT touch `daemon/crates/token-economics/` — that's T-003.
- DO NOT add new crate dependencies — no edits to any `Cargo.toml`.
- DO NOT modify `daemon/crates/shared-types/` or `daemon/crates/agent-adapter/` —
  AskAgentRequest extension is T-011's job.

## Tests you write

Add a unit test alongside `is_supported_agent_name` (the file already has tests at ~line 432):

```rust
#[test]
fn supports_deepseek_name() {
    assert!(super::is_supported_agent_name("deepseek"));
    assert!(super::is_supported_agent_name("gemini"));
    assert!(super::is_supported_agent_name("codex"));
    assert!(!super::is_supported_agent_name("claude"));
    assert!(!super::is_supported_agent_name(""));
}
```

Extend `daemon/crates/triumvirate/tests/integration_http.rs` (the file exists and already
hosts the daemon HTTP integration tests). Add a test that calls the `/status` endpoint and
asserts the response JSON's `supported_agents` array contains `"deepseek"`. Use the existing
test setup patterns in that file (look for prior `/status` tests if any; otherwise mirror
the daemon-startup pattern from neighbouring tests). Do NOT create a new file.

## Reality test (what "done" means)

1. `cargo check --workspace` exits 0.
2. `cargo test -p mcp-bridge supports_deepseek_name` passes
   (asserts `is_supported_agent_name("deepseek")==true` AND `("claude")==false`).
3. **The extended `/status` integration test in `integration_http.rs` asserts
   `"deepseek"` is in the response's `supported_agents` array.**
4. **TWO separate assertions — one per error path** (these are independent code paths;
   one assertion alone leaves the other path stale):
   - **4a.** A SESSION-SPAWN request with an unknown agent (e.g. `"fake-agent"`) returns
     an error whose supported-agents list explicitly contains `"deepseek"` — covers
     `main.rs ~line 2008` (the session-spawn error text).
   - **4b.** A `POST /ask-agent` HTTP request with an unknown agent in the body returns an
     error response whose supported-agents list explicitly contains `"deepseek"` — covers
     the `agent_exec.rs ~line 247` error text via its public HTTP surface (the
     `execute_ask_agent` function itself is `pub(crate)` and not accessible from external
     integration tests; `/ask-agent` is the correct test target).
   A stub that flips only the gate, or that updates only one of these two error texts,
   would FAIL. Both tests will need an in-process daemon spawn (use the patterns in
   `daemon/crates/triumvirate/tests/integration_http.rs`); they should be `#[ignore]`-gated
   if they require longer setup than the existing default tests tolerate.
5. `cargo test --workspace` exits 0 — **no Gemini/Codex regressions** (the
   blast-radius guard; a passing local test that broke other tests is NOT done).

## Closing ceremony — all three steps are REQUIRED

After your code passes `cargo check --workspace` AND `cargo test --workspace`:

### N-2. COMMIT
Use exactly this commit-message format (the regex `^feat\(deepseek\): T-001` will match):

```
feat(deepseek): T-001 — recognize deepseek as a top-level agent surface (REQ-DS-001/013/016/022)

Surface-wiring only — no dispatch routing, no runner. Every gate hardcoded
to gemini/codex now also accepts "deepseek". After this commit, ask_agent
{agent:"deepseek"} reaches the routing layer (which will error with "no
deepseek backend" until T-012 lands — expected).

Co-Authored-By: <model> <noreply@…>
```

### N-1. WRITE THE SENTINEL

Create `.triumvirate/TASK_COMPLETE.json` in the worktree root:

```bash
mkdir -p .triumvirate
cat > .triumvirate/TASK_COMPLETE.json <<EOF
{
  "task_id": "T-001",
  "commit_sha": "$(git rev-parse HEAD)",
  "result": "ok",
  "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "commit_message": "feat(deepseek): T-001 — recognize deepseek as a top-level agent surface (REQ-DS-001/013/016/022)"
}
EOF
```

### N. NOTIFY THE DAEMON VIA HTTP

```bash
curl -sf -X POST \
  -H "Authorization: Bearer $TRIUMVIRATE_TOKEN" \
  -H "Content-Type: application/json" \
  --data-binary @.triumvirate/TASK_COMPLETE.json \
  "http://localhost:$TRIUMVIRATE_HTTP_PORT/abe/task-complete" \
  || echo "HTTP notify failed — sentinel and commit remain as fallback"
```

After step N you are DONE. Do not run additional verification. Do not explore further.
Do not start new work. The daemon has three independent signals now and will mark T-001 done.

## If you get blocked

Use the Execution Contract's blocked protocol — single concrete blocker, evidence, proposed
fix, then STOP. Don't invent workarounds. Don't broaden scope.

```
blocked_on: <single concrete blocker>
task: T-001
evidence: <command + output summary, max 5 lines>
proposed_fix: <single action you would take>
```
