# Seven Primes retrospective and lessons

Curated from the private planning archive. The full archive (specs, dispatch briefs, peer verdicts) stays out of this
public repo.

## Outcome
- **Seats.** Kimi (Moonshot), Mistral (Vibe) and Qwen (QwenCloud) are live Triumvirate seats. They answer through
  ask_agent, named sessions, jury and `doctor --live`, and run read-only under a macOS seatbelt plus a repo audit.
- **Gating.** Each call is gated per (seat, surface, origin), with one call per seat at a time. Quota and suspension are handled, and sight-gated reviews work.
- **Writer seats (W-1).** `triumvirate seat-run` lets the seats write in git worktrees for ABE and fleet, under a
  worktree-only seatbelt and an audit of the parent tree and git metadata. Kimi and Qwen are proven live. Mistral is
  mock-tested only.
- **PostHog (P-1).** Every new path emits a `tv_seat_*` event with bounded values only. These were read back from PostHog Cloud on a tagged live run.

## What it cost
- **Ceremony.** The full plan-review ceremony (blind verifiers, bake-offs) cost a day with no working seat. The trimmed
  loop took the seats from zero to live in one night. That loop is a three-peer brief review, one builder, tests with a
  mutation kill rate, and the validator. Default to the trimmed loop and ship a live-smoked slice early.
- **Size.** About 30k lines landed, more than half of them tests.

## What actually found the defects
- **Live calls, not mocks.** Every vendor-shape defect surfaced on the first real call and was invisible to fixture tests built from vendor docs:
  - Kimi's agent file needs `description`.
  - Qwen needs `--auth-type`, has its own plan model list, and reports its session id in `system/init`.
  - Each seat keeps its read evidence somewhere different.
  - Vibe's docs contain placeholders.
  - The daemon's PATH lacks the seats.
- **Three-peer brief review.** It caught real gaps in every brief: cross-process locking, gitignored writes, test seams.
- **The new seats reviewing real code.** Mistral's REJECT found the only real defect in its review target.
- **Full-scope reruns.** A full `--workspace --tests` run caught a broken test that the narrower landing suite missed.
- **The first live fleet run.** It found that a gitignored `.triumvirate/` broke the writer commit step, now fixed with a regression test.

## Lessons
1. **Check that a package is alive before naming it.** A recommended install had been archived. Check the repo's archived flag and latest release first.
2. **Discard ungrounded search answers.** Several confident ones were fiction. Answer from vendor source or the vendor's own e2e fixtures.
3. **A mode named "plan" is not read-only.** Vibe `--agent plan` applied edits. Containment is an allowlist plus the OS sandbox plus a post-run audit, never a mode label.
4. **A server-side rollout can break a flag combination.** Pin the implementation selector (`VIBE_CLI=python`) and assert it in doctor.
5. **Read the plan's usage policy before designing unattended surfaces.** Gate per seat and per surface.
6. **For every claimed control, find the line that enforces it.** A gate that exists only in prose is documentation with a test.
7. **Keep doc lines under about 200 characters.** Very long lines truncate reviewers' file readers.
8. **Record the exact product, key format, env var and endpoint together.** Two vendor products shared one plan name.
9. **A red result counts only when the output names the failing test.** A shell word-splitting bug made every mutation look "caught" while nothing ran. Run multi-word commands from bash scripts.
10. **Run every gate once against main before relying on it.** Gates written for an ideal codebase could not pass on this one.
11. **A test step counts only when it shows `test result: ok. N passed` with N > 0.** Empty output is a failure.
12. **Assemble fake secrets at runtime from pieces, never as literals, in tests too.** Gitleaks scans every branch commit.
13. **A passed planning gate is not a passed build.** Run the per-task review before every task.
14. **A generator never deletes a directory that can hold what it did not write.** Commit audit evidence in the same step.
15. **Never accept a filtered test run as proof for code that touches process-global state.** Run the whole crate.
16. **Evidence runs use a private cargo target dir.** Parallel agents sharing one target dir overwrite each other's test binaries.
17. **An empty result from a measuring tool is a FAIL.** A mutation run that matched no files exited 0.
18. **Private target dirs must not sit under a temp root.** The seatbelt allows writes there, which fails containment tests falsely.
19. **Map every builder diff to the brief yourself.** A builder pass reporting only "did not commit" hid missing tests and a scope violation. Run its verify command yourself.
20. **The landing suite is `cargo test --workspace --exclude pantheon --tests` plus clippy.** Narrower suites skipped crates' integration tests.
21. **Gate commits with `&&` on the check's exit status, never `;`.** Live fixtures use neutral names, never secret, token or key wording, because a model may refuse to read them.
22. **Push with a private `CARGO_TARGET_DIR`.** The pre-push hook otherwise compiles in a shared target. A hook failure
    naming code that plainly exists is a target-dir collision until proven otherwise.

## Open items
- **Ledger writes into reviewed repos.** The ledger appends `.triumvirate/` to a reviewed repo's .gitignore and writes ledger.db into crate dirs during tests. This predates Seven Primes.
- **Mistral as a live writer.** It is not live-proven: the operator's permission classifier blocked the probe.
- **Prompt text in PostHog.** The pre-existing `$ai_generation` event sends prompt text (`$ai_input`). A follow-up makes this opt-in.
- **A grok `u_b_18` env-lock flake.** Old code, separate fix.
