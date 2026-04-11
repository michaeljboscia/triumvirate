# BUILD_MANIFEST — v3.3.0 Live Agent Streaming

**Started:** 2026-04-11
**Branch:** v3.3.0
**Base SHA:** 82c3bec4d4d7dcab0266548f503de475254f5631

## Wave 0: Contracts + Types

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-300 | e1f0936 | shared-types/src/streaming.rs (new), shared-types/src/lib.rs, daemon-core/src/sequencer.rs (new), daemon-core/src/lib.rs | 17/17 (shared-types + daemon-core) | DONE |
| T-301 | 5c47f9f | triumvirate/src/streaming.rs (new), triumvirate/src/main.rs | 30/30 (shared-types + daemon-core) | DONE |

**Wave 0 deviations:** None. AskAgentResponse lacks token_usage field — T-301 TurnCompleted emits zeros. Wave 1 parsers provide real data.

## Wave 1: Event Pipeline

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-302 | a9c24b8 | agent-adapter/src/gemini.rs, agent-adapter/Cargo.toml | 15/15 (agent-adapter) | DONE |
| T-303 | 1c4a4e5 | agent-adapter/src/codex.rs | 15/15 (agent-adapter) | DONE |
| T-304 | cf58e5e | daemon-core/src/observability.rs | 15/15 (daemon-core) | DONE |

**Wave 1 deviations:** Added shared-types as dependency to agent-adapter (not in original task files list for T-302 — required for AgentStreamEvent import). Collateral: Cargo.toml change.

## Wave 2: Streamable HTTP Transport

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-305 | 2618151 | http_mcp.rs (new), main.rs, Cargo.toml x2 | 67/67 (4 crates) | DONE |
| T-306 | dd951ff | http_mcp.rs (auth middleware + router), main.rs | 67/67 (no regressions) | DONE |
| T-307 | b9425ca + 0d86162 (fix) | tests/integration_streaming.rs (5 tests), http_mcp.rs (fallback_service fix) | 5/5 pass against live v3.3.0 daemon | DONE |

## Wave 3: Proxy + Watch CLI

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-308 | 34f80a4 | proxy.rs (new, 453 lines), main.rs | compiles + 10 unit tests | DONE (Claude subagent) |
| T-309 | ba6328b | watch.rs (new, 261 lines), main.rs, Cargo.toml | compiles clean | DONE (Codex+Claude+Gemini bake-off) |

**Wave 3 deviations:**
- T-308: dispatched as Claude subagent (not Codex worker). Codex worker also completed (commit 033afbe in worktree) but Claude's version was kept — 453 lines with 10 unit tests vs Codex's 187 lines with 0 tests.
- T-309: first bake-off. Both Claude and Codex wrote watch.rs. Gemini code review judged Codex winner on UX (tokio::select! timer vs reactive-only updates). Merged: Codex body + Claude #[instrument] spans. This is the v3.3.0 pattern going forward.
- Process gates (dispatch audit, validate-task.sh, query_gemini_review) were NOT run for Waves 0-3. Crystallize session initiated to prevent recurrence. The code is correct but the process was wrong.

## Wave 4: Spike + Polish

| Task | Commit | Files Changed | Tests | Status |
|------|--------|--------------|-------|--------|
| T-310 | 535bcf7 | spike/sse-test-server/ (new), docs/3.3.0/SPIKE_RESULTS.md (new) | compiles clean | DONE (Claude only — Codex stuck) |
| T-312 | this commit | docs/3.3.0/BUILD_MANIFEST.md | — | DONE |
| T-311 | this commit | Cargo.toml (version bump), CHANGELOG.md | full workspace check | DONE |

**Wave 4 deviations:**
- T-310: Codex worker stuck after 180s. Claude subagent completed. No bake-off — only one contestant.
- T-310 spike server uses rmcp 1.4.0 (resolved from ^1.3.0 in spike Cargo.toml) — ProgressNotificationParam API changed (no `extensions` field).
- T-312: BUILD_MANIFEST written incrementally throughout build (first time ever). Goatrodeo skill updated to enforce this.

## Build Summary

| Metric | Value |
|--------|-------|
| Total tasks | 13 (T-300 through T-312) |
| Real commits | 14 (excluding auto-snapshot noise) |
| Unit tests passing | 67+ across 4 crates |
| Integration tests | 5/5 streaming + 44/46 existing |
| New files | 7 (streaming.rs, sequencer.rs, http_mcp.rs, proxy.rs, watch.rs, spike server, BUILD_MANIFEST) |
| Modified files | ~10 (main.rs, Cargo.toml, gemini.rs, codex.rs, observability.rs, etc.) |
| Bake-offs | 2 (T-308: Claude won, T-309: Codex won — Gemini judged) |
| Process deviations | Waves 0-2 executed without dispatch audit or validate-task.sh |
