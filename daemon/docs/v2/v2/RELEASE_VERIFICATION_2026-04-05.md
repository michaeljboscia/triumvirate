# Release Verification — 2026-04-05

## Scope

Validation pass for Triumvirate v2 branch `feat/phase-0` toward v0.1.0/1.0 readiness.

## Automated Gates

All of the following passed from `/Users/you/projects/triumvirate/daemon`:

- `cargo check`
- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo build`
- `cargo build --release`
- `npm run build` (from `daemon/frontend`)

## Runtime Smoke (release binary)

Executed release daemon binary and validated live endpoints:

- `GET /api/health` -> `{"status":"ok","agents":{"claude":"ready","gemini":"ready","codex":"ready"}}`
- `GET /metrics` -> Prometheus exposition text (`# HELP`, `# TYPE`, metric series present)
- `GET /api/costs` -> cost attribution JSON (`summary` + `agents` objects present)
- `GET /` -> embedded frontend served (index payload returned)

## Functional Coverage Implemented in This Build Window

- Dashboard header + quota bars and phase 5 panel expansion (quota, memory, workflow, merge resolver).
- Cost attribution panel + backend `/api/costs` endpoint.
- Prometheus metrics endpoint `/metrics` with runtime instrumentation.
- Build-time Svelte embedding flow integrated into Rust compile path.
- Optional Langfuse turn ingestion client (disabled by default; config-gated).
- Flaky env-var connector tests stabilized under parallel test execution.

## Remaining Manual Success Criteria (SPEC.md)

The following are partially or fully manual/user-observational and should be signed off in interactive usage:

1. 30-second dashboard comprehension by a fresh observer.
2. Session-close notes quality verified against real task/git/tool history in a real workflow.
3. Multi-agent simultaneous self-activation behavior in a realistic collaborative scenario.
4. Full fleet command scenario: “3 Claudes, 2 Codexes, 1 Gemini” with end-to-end human-observed coordination/merge.

## Current Conclusion

Code-level and build-level release gates are passing. Runtime smoke on the release binary is passing.
