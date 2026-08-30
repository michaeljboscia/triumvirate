# SQLite Concurrency Stress Test

Run ID: `smoke-mixed`
Profile: `MixedRw { read_pct: 25 }`
Workers: `30`
Duration: `20` seconds
Started (UTC): `2026-04-19 09:09:06.314595 UTC`
Finished (UTC): `2026-04-19 09:09:35.333813 UTC`

## Latency

- p50: `0.775` ms
- p95: `44.767` ms
- p99: `96.255` ms
- p99.9: `97.151` ms

## Reliability

- SQLITE_BUSY retries: `0`
- BUSY rate: `0.0000`
- Successful ops: `107`
- Failed ops: `0`

## WAL and Host

- WAL peak: `1.945` MB
- Harness process CPU: `0.21` %
- System load avg (1m): `9.83`
