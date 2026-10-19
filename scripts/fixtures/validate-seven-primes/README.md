# validate-seven-primes fixtures

Used only by `bash scripts/validate-seven-primes-task.sh --self-test` (Seven Primes T-000, REQ-010).

- `template/` is copied into a scratch git repo as its whole tree. Its `daemon/` is a three-crate workspace
  (fx depends on fy, fy depends on fz, path dependencies only, so no network) that plays the part of the real daemon.
- `variants/` holds deliberately bad files the self-test swaps in: a failing test plus a clippy warning, and a
  semgrep WARNING pattern. They live outside any `daemon/` directory here, so the real repo's floor (Rust scope is
  `daemon/**/*.rs`) never scans them.

The fake secret for the gitleaks case is assembled at runtime by the self-test and never committed.
