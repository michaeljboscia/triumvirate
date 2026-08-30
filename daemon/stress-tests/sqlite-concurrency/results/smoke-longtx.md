# SQLite Concurrency Stress Test

Run ID: `smoke-longtx`
Profile: `LongTx { hold_ms: 100 }`
Workers: `30`
Duration: `20` seconds
Started (UTC): `2026-04-19 09:08:35.950508 UTC`
Finished (UTC): `2026-04-19 09:09:05.857164 UTC`

## Latency

- p50: `141.823` ms
- p95: `2738.175` ms
- p99: `3258.367` ms
- p99.9: `3364.863` ms

## Reliability

- SQLITE_BUSY retries: `0`
- BUSY rate: `0.0000`
- Successful ops: `103`
- Failed ops: `0`

## WAL and Host

- WAL peak: `2.499` MB
- Harness process CPU: `0.48` %
- System load avg (1m): `11.71`
