# BUG REPORT — ABE red-team enforcement: stub-detection is not blocking the third compliance scenario

**STATUS: RESOLVED 2026-05-26** — root-caused as a pattern-set drift; validator's
`stub_patterns` didn't match the test's `// pending: stub` marker. Fixed by adding
`"// pending"` and `"// stub"` to the pattern list in
`daemon/crates/triumvirate/src/abe/post_exit_validator.rs`.

Approach decided via a 3-agent council (DeepSeek v4-pro, Codex, Gemini) — all
three independently recommended adding comment-prefixed patterns to avoid
false positives on legitimate identifiers like `pending_orders` / `stub_module`.
Two of three (DeepSeek + Codex) wanted no colon suffix; Gemini wanted a colon.
No-colon won 2-1 to also catch realistic markers like `// pending implementation`.
3-agent council cost: ~$0.005.

Test now passes:
`cargo test --bin triumvirate -- tests::abe_red_team_enforcement_blocks_non_compliant_worker --exact`
Full workspace: **172/172 triumvirate tests pass** (was 171/172 with this as
the sole failure).

---

## Original bug report

**Discovered:** 2026-05-26 during the DeepSeek T-001 build, while running `cargo test --workspace` as the blast-radius guard.
**Tree where reproduced:** confirmed on both `f529123` (pre-T-001 spec/dispatch commit) and `765086e` (post-T-001) — i.e. PRE-EXISTING; not caused by the DeepSeek work.
**Branch:** `spec/deepseek-integration-v1` (but bug exists wherever the ABE red-team test lives — also on `main`).
**Severity:** Medium — the test that asserts the daemon blocks non-compliant workers is *itself* failing, which means a real safety guard around stub-shipping is currently disabled (or the test has drifted from the enforcement code).

## Symptom

```
test tests::abe_red_team_enforcement_blocks_non_compliant_worker ... FAILED

thread 'tests::abe_red_team_enforcement_blocks_non_compliant_worker' panicked at
crates/triumvirate/src/main.rs:5935:9:
assertion failed: !matches!(s3, shared_types::TaskStatus::Completed)
```

The test dispatches three intentionally-non-compliant worker scripts and asserts each one ends up
NOT in `TaskStatus::Completed` (i.e. the red-team enforcement caught it):

```rust
let s1 = dispatch_and_expect_failed(forbidden_file_script, "T-016A").await?;
assert!(!matches!(s1, TaskStatus::Completed));                  // ✅ passes — forbidden-file caught
let s2 = dispatch_and_expect_failed(bad_commit_script, "T-016B").await?;
assert!(!matches!(s2, TaskStatus::Completed));                  // ✅ passes — bad-commit caught
let s3 = dispatch_and_expect_failed(stub_script, "T-016C").await?;
assert!(!matches!(s3, TaskStatus::Completed));                  // ❌ FAILS — stub gets through
```

s1 (forbidden file violation) and s2 (bad commit format) are caught correctly. **s3 (stub
detection) is not** — the worker that shipped a stub gets marked `Completed` instead of being
rejected.

## Reproduction

```bash
cd daemon
cargo test --bin triumvirate -- \
  tests::abe_red_team_enforcement_blocks_non_compliant_worker --exact --nocapture
```

~2-second runtime; reproduces 100% on the current `spec/deepseek-integration-v1` HEAD and on
the pre-DeepSeek tree.

## Code pointers (where to look)

- **The test:** `daemon/crates/triumvirate/src/main.rs:5928-5935` (the assertion that fires).
- **`dispatch_and_expect_failed` (the test helper):** in `daemon/crates/triumvirate/src/main.rs`
  (grep `fn dispatch_and_expect_failed`).
- **`stub_script` (the input that should be rejected):** built locally in the test body — search
  `stub_script` ~lines above 5935. It writes a Rust source containing `todo!()` /
  `unimplemented!()` / similar stub markers that the daemon's red-team validator is supposed to
  detect after the worker commits.
- **The ABE red-team enforcement path:** `daemon/crates/mcp-tools/src/abe/*` (the ABE module).
  Likely the stub-detection check lives in a validator that's invoked on the worker's diff
  before marking the task `Completed`.

## Likely fix surface

One of:
1. The stub-detection regex/AST scan no longer matches the markers in `stub_script` (recent
   refactor missed updating the validator's pattern set).
2. The validator runs but its result isn't being routed into the `TaskStatus` outcome — i.e. it
   detects the stub but completes the task anyway.
3. The validator is gated behind a feature/env that's off in the test environment.

A `git log -p crates/mcp-tools/src/abe/ -- crates/triumvirate/src/main.rs` around the stub-detection
code, since the last passing CI run, will narrow it. Likewise `git blame` on the validator's
detector to see if the matcher recently changed.

## Why this matters

The whole point of the red-team test is to assert that workers who ship stubs (`todo!()`,
`unimplemented!()`, hardcoded byte-hasher producing fake vectors, etc.) get **rejected** at
commit-validation time. If the test fails because the daemon completes them anyway, the
safety guard is disabled in production code too — workers can currently ship stubs and have
their tasks marked Completed.

This is exactly the Pythia v2 failure mode the goatrodeo spec keeps referencing — "stubs that
satisfied the type system." The red-team test exists to keep that from happening; right now
it's also the canary that this guard is broken.

## Operator notes

- No new failure introduced by the DeepSeek work — confirmed by running the test on the
  pre-T-001 tree (stash + cargo test against `f529123`).
- T-001 ships green on every other test in the workspace; this bug is independent and can be
  fixed in its own PR.
- Sibling bug for the daemon's `/session/ask` failure remains tracked at
  `daemon/docs/bugs/2026-05-25-daemon-session-ask-intermittent-failure.md`.
