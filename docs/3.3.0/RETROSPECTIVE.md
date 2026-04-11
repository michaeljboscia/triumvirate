# Retrospective — v3.3.0 Live Agent Streaming

**Date:** 2026-04-11
**Sprint duration:** ~6 hours (goatrodeo + build + postrodeo)
**Branch:** v3.3.0

## Completion

| Metric | Value |
|--------|-------|
| REQs pass | 22/25 |
| REQs partial | 1 (REQ-H05 — streaming unverified with live agent) |
| REQs manual | 2 (REQ-H08, REQ-K01 — need interactive testing) |
| Tasks | 13/13 complete |
| Commits | 14 real (excluding auto-snapshot noise) |
| Unit tests | 67/67 |
| Integration tests | 117/118 (1 pre-existing agent timeout) |
| New streaming tests | 5/5 |
| Bake-offs | 2 (T-308: Claude won, T-309: Codex won) |
| BUILD_MANIFEST | ✅ First time in 7 sprints |

## What Went Well

1. **7-round goatrodeo** produced a battle-tested spec. Phase 1 dropped early (Claude Code doesn't render progress notifications). Smart proxy architecture evolved through 3 rounds of refinement.

2. **Multi-model bake-off** — the breakthrough of the sprint. Claude + Codex write the same task, Gemini judges for $0.001. Produced better code than either alone (T-309: Codex's tokio::select! timer + Claude's #[instrument] spans).

3. **Gemini sweep in postrodeo** caught "Normalization of Deviance" — a blind spot both twins missed. Three-model review > two-model review.

4. **Incremental BUILD_MANIFEST** — created at build start, appended after each wave. Goatrodeo skill updated to enforce this mechanically.

5. **Research-first architecture** — 12 quicksearches before the goatrodeo found that Claude Code ignores MCP progress notifications. Saved an entire phase of wasted implementation.

## What Went Wrong

1. **Process gates completely skipped** for Waves 0-2. No dispatch audits, no validate-task.sh, no query_gemini_review. Claude did the work itself instead of dispatching workers. /crystallize initiated but not completed.

2. **Wrong dispatch mechanism** — used Claude subagents (same rate limit) instead of Codex workers (separate rate limit). Two workers produced nothing before the error was caught.

3. **REQ-H05 unverified** — the core streaming feature is plumbed but never tested with a live agent call over the HTTP MCP path. This is the highest-risk gap.

4. **Gemini MCP costs not tracked** — 12 quicksearches + 1 code analysis used the GEMINI_API_KEY (per-token billing), not the CLI subscription. Cost was ~$0.001 per call but the billing path wasn't understood until mid-sprint.

## Lessons

1. **Bake-off pattern works.** Codex writes better UX code. Claude writes better observability. Gemini correctly picks the winner. Merge produces superior output. Baked into /goatrodeo Phase 5.4.

2. **Gemini sweep catches blind spots.** The "Normalization of Deviance" finding was worth the entire postrodeo. Baked into /postrodeo Phases 4.2 and 5.3.

3. **Model switching at phase boundaries.** Opus for thinking (goatrodeo), Sonnet for doing (build). Saves ~5x token budget. Memory created but not mechanically enforced yet.

4. **BUILD_MANIFEST must be created BEFORE the first task, not as a late-stage task.** Goatrodeo skill updated. This lesson took 7 sprints to crystallize.

5. **Know your billing paths.** Gemini MCP ≠ Gemini CLI. One costs money, the other is subscription. Both work. Check before assuming.

## Process Improvements Shipped

| Improvement | Where | Commit |
|------------|-------|--------|
| BUILD_MANIFEST mandatory creation at Phase 5 start | /goatrodeo | goatrodeo.md |
| Bake-off dispatch pattern (Phase 5.4) | /goatrodeo | goatrodeo.md |
| Gemini sweep on deviation review (Phase 4.2) | /postrodeo | postrodeo.md |
| Gemini sweep on code review (Phase 5.3) | /postrodeo | postrodeo.md |
| Model switching advisory at Phase 4→5 boundary | memory | feedback_model_switching.md |
| BUILD_MANIFEST enforcement hook | hooks | pre-tool-use-build-manifest-gate.sh |

## Open Items

1. Complete /crystallize for process gate skipping
2. Manual spike test (REQ-K01) — run sse-test-server, register in Claude Code
3. Manual HTTP MCP test (REQ-H08) — `claude mcp add --transport http`
4. Live agent call over HTTP (REQ-H05) — verify streaming end-to-end
5. Pantheon v4.0 goatrodeo — Ratatui TUI spec
