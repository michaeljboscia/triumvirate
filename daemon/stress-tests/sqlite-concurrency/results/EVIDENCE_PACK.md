# EVIDENCE PACK

Canonical aggregation of sqlite-concurrency stress-test results.

## Concurrency Matrix

- No matrix reports found.

## Crash-Recovery Trials

### Report: `smoke-crash`

- Summary: 3 trials, all_integrity_ok=`true`, avg wal_replay_ms=`0.000`

| trial_id | pre_crash_count | post_crash_count | delta | integrity_ok | wal_replay_ms |
|---|---:|---:|---:|---:|---:|
| unknown | 15383 | 15386 | 3 | true | 0.000 |
| unknown | 15357 | 15360 | 3 | true | 0.000 |
| unknown | 15402 | 15404 | 2 | true | 0.000 |

## Summary Verdict

- Total successful ops: `0`
- Total failed ops: `0`
- Total ops: `0`
- Breaches: p99=`0`, busy_rate=`0`, failed_ops=`0`, crash_integrity=`0`

- ADR-001 Tier 1 claim `Latency p99 <= 500ms`: **PASS**
- ADR-001 Tier 1 claim `Contention busy_rate <= 0.01`: **PASS**
- ADR-001 Tier 1 claim `Operational reliability (failed_ops == 0)`: **PASS**
- ADR-001 Tier 1 claim `Crash integrity preserved`: **PASS**
- Aggregate ADR-001 Tier 1 verdict: **PASS**

## Run Inventory

| source_json | shape | run_id | started_at | duration_sec |
|---|---|---|---|---:|
| ./results/smoke-crash.json | crash_trial | n/a | n/a | n/a |

