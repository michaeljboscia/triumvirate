# dispatch_codex — Output Capture + Reasoning Effort Knob

**Status:** Spec, ready to implement. Tier 1 + Tier 4 of the post-`de02370` refactor plan.
**Scope:** Make `dispatch_codex` return real model output and let callers control reasoning effort.
**Out of scope:** Streaming JSON parser promotion (Tier 2), warm-session reuse (Tier 3) — separate specs.
**Cross-references:** Commit `de02370` (`fix(mcp-tools): add --skip-git-repo-check to dispatch_codex argv`); `agent_exec.rs:1496-1700` (reference implementation).

## Problem

`dispatch_codex` currently spawns `codex exec --full-auto --skip-git-repo-check <prompt>` and waits for the child to exit. After the trusted-dir fix in `de02370`, dispatches succeed (exit 0) but two defects remain:

1. **Empty `stdout` in `get_task_output`.** The argv lacks `--output-last-message <tmpfile>`, so the daemon has no clean channel to read the model's final answer. Raw piped stdout on `--full-auto` non-JSON mode mostly contains the codex banner, not the response.

2. **No way to control reasoning effort.** The codex profile defaults to `xhigh`, which burns 250-400s on trivial prompts. Callers can't opt down to `medium` or `low` for fire-and-forget tasks where 5-10s answers are sufficient.

The session path (`agent_exec.rs:1496-1506`) already implements `--output-last-message` correctly. Tier 1 is direct line-for-line parity. Tier 4 adds a new request field.

## Requirements

| ID | Requirement |
|----|-------------|
| REQ-DC-01 | `dispatch_codex` MUST pass `--output-last-message <tmpfile>` to codex-exec, with a per-call uuid-tagged path under `std::env::temp_dir()`. |
| REQ-DC-02 | After child exit with status 0, the daemon MUST read the tmpfile and store its contents as the task's stdout, so `get_task_output` returns the model's final answer. |
| REQ-DC-03 | Tmpfile MUST be deleted after read (success path) or after timeout/failure (cleanup path), no orphaned files in `/var/folders/.../T/` under any control flow. |
| REQ-DC-04 | The argv builder for both `dispatch_codex` and `ask_*` paths MUST live in a single shared helper, eliminating the drift that caused `de02370`. |
| REQ-DC-05 | `DispatchCodexRequest` MUST accept an optional `effort: Option<String>` field with values `low \| medium \| high \| xhigh`. |
| REQ-DC-06 | When `effort` is set, the argv MUST include `-c model_reasoning_effort=<value>` BEFORE `exec` (codex `-c` flags must precede the subcommand). |
| REQ-DC-07 | When `effort` is unset, behavior MUST match today's (codex profile default applies, no `-c` flag emitted). |
| REQ-DC-08 | Existing callers passing only `prompt` (no `effort`) MUST continue to work without code change. Backward compatibility is non-negotiable. |
| REQ-DC-09 | A regression test MUST verify that a dispatch with `effort: "medium"` completes in <60s for a trivial prompt on gpt-5.5, while the same dispatch without `effort` takes >120s. |

## Contracts

### Shared argv builder (new module)

```rust
// daemon/crates/mcp-tools/src/codex_argv.rs (new file)

pub enum CodexExecMode {
    Fresh,                         // dispatch_codex
    Resume { session_id: String }, // ask_agent / ask_session
}

pub struct CodexExecArgs {
    pub bin: String,
    pub args: Vec<String>,
    pub last_message_path: PathBuf,  // tmpfile to read after child exits
}

pub fn build_codex_exec_argv(
    base_command: (String, Vec<String>),  // from callbacks.codex_command()
    cwd: &str,
    message: &str,
    mode: CodexExecMode,
    effort: Option<&str>,                 // REQ-DC-05/06/07
) -> CodexExecArgs;
```

Behavior:
- `-c model_reasoning_effort=<value>` prepended only when `effort.is_some()`.
- `exec` subcommand always appended.
- `Mode::Resume` adds `resume --dangerously-bypass-approvals-and-sandbox <session_id>`.
- `Mode::Fresh` adds `--full-auto`.
- `--skip-git-repo-check` added when `!is_git_worktree(cwd)` (matches `agent_exec.rs:1496` semantics).
- `--json` added always (Tier 2 will consume; for now the daemon ignores extra event lines).
- `--output-last-message <uuid-tmpfile>` always.
- Final positional arg: the prompt/message.

### dispatch_codex output capture

```rust
// daemon/crates/mcp-tools/src/abe.rs (modified)

pub struct DispatchCodexRequest {
    pub prompt: String,
    pub cwd: Option<String>,
    pub sandbox: Option<DispatchSandbox>,
    pub timeout_sec: Option<u64>,
    pub effort: Option<String>,    // REQ-DC-05 — NEW
}
```

Completion path (currently in `tokio::spawn` block at `abe.rs:443+`):
1. Wait for child exit.
2. If exit 0: read `last_message_path`, pass contents to `tracker.complete_task(task_id, contents)`.
3. If exit != 0: read `last_message_path` if it exists (codex sometimes writes partial output before failing) plus stderr_tail (Tier 2), pass to `tracker.fail_task`.
4. In all cases (success, failure, timeout): unlink the tmpfile in a `defer`-style cleanup. REQ-DC-03.

## File changes

| File | Change |
|------|--------|
| `daemon/crates/mcp-tools/src/codex_argv.rs` | **NEW** — shared argv builder |
| `daemon/crates/mcp-tools/src/lib.rs` | Add `pub mod codex_argv;` |
| `daemon/crates/mcp-tools/src/abe.rs` | Replace lines 395-399 with `build_codex_exec_argv(...)`. Add tmpfile-read + cleanup in completion handler. Add `effort` field to `DispatchCodexRequest`. |
| `daemon/crates/triumvirate/src/agent_exec.rs` | Replace lines 1496-1506 with call to shared builder. REQ-DC-04. |
| `daemon/crates/mcp-bridge/src/lib.rs` | Update `DispatchCodexRequest` JSON schema to include optional `effort`. |
| `daemon/crates/mcp-tools/tests/dispatch_codex_test.rs` | **NEW** — integration test per REQ-DC-09. |

## Tasks

<task id="T-001" req="REQ-DC-04" wave="0" depends="">
  <description>Create codex_argv.rs with shared argv builder</description>
  <files>daemon/crates/mcp-tools/src/codex_argv.rs, daemon/crates/mcp-tools/src/lib.rs</files>
  <contract>build_codex_exec_argv(base, cwd, message, mode, effort) returns CodexExecArgs</contract>
  <verify>cargo check -p mcp-tools</verify>
</task>

<task id="T-002" req="REQ-DC-04" wave="1" depends="T-001">
  <description>Migrate agent_exec.rs lines 1496-1506 to call build_codex_exec_argv</description>
  <files>daemon/crates/triumvirate/src/agent_exec.rs</files>
  <contract>Identical argv to current behavior; ask_agent regression-tested with one trivial prompt</contract>
  <verify>cargo test -p triumvirate ask_agent_smoke; manual mcp__triumvirate__ask_agent returns OK</verify>
</task>

<task id="T-003" req="REQ-DC-01,REQ-DC-02,REQ-DC-03" wave="1" depends="T-001">
  <description>Migrate abe.rs dispatch_codex argv to shared builder; add tmpfile read + cleanup</description>
  <files>daemon/crates/mcp-tools/src/abe.rs</files>
  <contract>Successful dispatch returns last_message text via get_task_output; tmpfile unlinked in all exit paths</contract>
  <verify>cargo test -p mcp-tools dispatch_codex_returns_output</verify>
</task>

<task id="T-004" req="REQ-DC-05,REQ-DC-06,REQ-DC-07,REQ-DC-08" wave="2" depends="T-003">
  <description>Add effort field to DispatchCodexRequest; thread through to argv builder</description>
  <files>daemon/crates/mcp-tools/src/abe.rs, daemon/crates/mcp-bridge/src/lib.rs</files>
  <contract>effort=Some("medium") emits -c model_reasoning_effort=medium before exec; effort=None unchanged from today</contract>
  <verify>cargo test -p mcp-tools dispatch_codex_effort_argv</verify>
</task>

<task id="T-005" req="REQ-DC-09" wave="2" depends="T-004">
  <description>Integration test: trivial dispatch with effort=medium &lt;60s, without effort &gt;120s</description>
  <files>daemon/crates/mcp-tools/tests/dispatch_codex_test.rs</files>
  <contract>Both assertions pass; test gated on TRIUMVIRATE_INTEGRATION_TEST=1 env var (real codex CLI required)</contract>
  <verify>TRIUMVIRATE_INTEGRATION_TEST=1 cargo test -p mcp-tools --test dispatch_codex_test</verify>
</task>

## Verification

| Level | Check |
|-------|-------|
| L1 (per task) | `cargo check -p &lt;crate&gt;` after each task |
| L2 (per wave) | All ask_agent / ask_session tests still pass after Wave 1 (no regression) |
| L3 (per plan) | `cargo test --workspace` clean; manual `mcp__triumvirate__dispatch_codex` returns non-empty output via `get_task_output` |
| L4 (pre-ship) | 10× parallel dispatch smoke test with `effort: "medium"`, all returning answers in &lt;90s |

## Acceptance criteria (paste into TEST_PLAN.md)

| REQ | Test |
|-----|------|
| REQ-DC-01 | Inspect daemon log for the dispatched task — `--output-last-message /var/folders/.../triumvirate-codex-last-message-*.txt` MUST appear in the spawn argv |
| REQ-DC-02 | `mcp__triumvirate__dispatch_codex({prompt: "Reply OK_TEST"})` → poll `get_task_output` → MUST return stdout containing `OK_TEST` |
| REQ-DC-03 | Run 10 dispatches; after all complete, `find $TMPDIR -name 'triumvirate-codex-last-message-*'` MUST return zero matches |
| REQ-DC-04 | `git grep "args.push.*output-last-message"` returns exactly ONE call site (the shared helper); was 1 before refactor in agent_exec.rs |
| REQ-DC-05 | dispatch_codex MCP schema (via `tools/list`) shows optional `effort` field |
| REQ-DC-06 | With `effort: "medium"`, daemon log shows `-c model_reasoning_effort=medium` BEFORE the `exec` token in the argv |
| REQ-DC-07 | Without `effort`, daemon log MUST NOT contain `-c model_reasoning_effort` |
| REQ-DC-08 | Existing callers (any test or daemon code passing only `prompt`) continue to compile and run unchanged |
| REQ-DC-09 | Same trivial prompt: `effort: "medium"` &lt;60s; default xhigh &gt;120s — both on gpt-5.5 |

## Risks

| Risk | Mitigation |
|------|------------|
| Shared argv builder regresses ask_agent | T-002 runs ask_agent smoke before continuing to T-003; bisect-friendly |
| `-c model_reasoning_effort` flag rename in future codex CLI | Probe via `codex --help` at daemon boot (already done in `057080f` for `--full-auto`); add fallback path |
| Tmpfile cleanup leaks on panic | Use `tempfile::NamedTempFile` instead of manual `std::env::temp_dir().join(...)` — auto-cleans on drop |
| Effort value not validated | Validate at request boundary: enum `Effort { Low, Medium, High, XHigh }` deserialized from string; reject unknown values with clear error |

## Estimated effort

| Task | Estimate |
|------|----------|
| T-001 | 30 min |
| T-002 | 30 min |
| T-003 | 60 min |
| T-004 | 30 min |
| T-005 | 30 min |
| **Total** | **~3 hours** |

Suitable for a single Codex worktree dispatch (`dispatch_codex_worktree`) with the full task list as the briefing — exactly the kind of mechanically-scoped work that path is designed for. Cleanly tests the just-fixed dispatch path itself by being the first non-trivial workload to use it post-`de02370`.
