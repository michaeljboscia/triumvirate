# P-1 PostHog instrumentation: live evidence (2026-10-19, release build of posthog-p1 12fb185e)

Tagged run `p1live-1792397220` (TRIUMVIRATE_TELEMETRY_RUN_TAG), real CLIs, real seat homes, PostHog Cloud project 523080
(host us.i.posthog.com; key read from the triumvirate env block by a wrapper that never prints it). Read back with HogQL
filtered to the tag; the capture call's status was never the proof.

| Event | Properties (read back) | Count |
|---|---|---|
| tv_seat_doctor_probe | kimi / mistral / qwen, tv_outcome pass | 3 |
| tv_seat_writer_run | kimi, committed, surface abe_dispatch | 1 (arrived after ingestion lag) |
| tv_seat_gate_refused | mistral, seat_not_configured, abe_dispatch, unattended | 1 |
| tv_seat_writer_run | mistral, failed, abe_dispatch | 1 |

Not exercised live (cannot be forced safely against real accounts; proven by call-site and capture-server tests):
quota, suspension, seatbelt_unavailable, escapes, timeout, killed, generation_conflict, lock wait, served-model mismatch.

Pre-existing finding (not changed by P-1): the older `$ai_generation` event sends prompt text (`$ai_input`) and the gate
message (`tv_detail`) to PostHog by design. Every new `tv_seat_*` event is free of both leak sentinels.
