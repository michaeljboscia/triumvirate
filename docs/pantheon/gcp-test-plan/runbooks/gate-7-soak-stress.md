# Gate 7 — Soak + Stress (Long-Session Stability)

**Purpose:** Validate Pantheon's operational stability under sustained load. Short bursts (Gates 1-5) prove capability; Gate 7 proves durability. Catches KV cache drift, schema-validity decay, retry storms, memory leaks, and other failure modes that only emerge after hours of continuous operation.

**GCP config:** Reuses Gate 5's full trinity composition OR a smaller single-VM setup depending on what's being soaked
**Cost:** ~$20-40/hr × 4-8 hrs = **~$100-300 per session**
**Duration:** 4-8 hour sustained runs (hard cap per run)
**Pre-committed decision rule:** see `30-DECISION-RULES.md` → Decision 8 (production readiness)

---

## Why this gate is last AND most expensive

Every other gate is a burst test. Gate 7 asks: "Does Pantheon still work on hour 6?" This matters because:

- **Customer production environments run 24/7**, not in 2-hour demos
- **Cumulative KV cache pressure** under continuous tool-use eventually degrades output quality in subtle ways
- **Memory leaks** in vLLM / Triumvirate only surface after many hours
- **Schema-validity decay** — local models sometimes get "sloppier" after long sessions (tool-call JSON gets malformed)
- **Retry storms** — when one component hiccups, does Triumvirate cascade-retry appropriately or pile up?
- **Log/metric volume** at scale — does the evidence pipeline handle a 4-hour run's data volume?

Gate 7 is the final ship gate before sovereign customer deployment.

---

## Three sub-gates (run as needed, not all at once)

### Sub-gate 7a — KV cache pressure soak (4 hrs)
Single Zeus + Athena + Vulcan running 50+ agent tasks sequentially with long-context memory.

### Sub-gate 7b — Concurrent-load sustained (4 hrs)
Full parallel worker pool, continuous dispatch at peak concurrency. Measures degradation curves.

### Sub-gate 7c — Fault injection + recovery (2-4 hrs)
Deliberately break things (kill containers, saturate network, expire auth), verify Triumvirate recovers.

---

## Hypotheses being tested

### H-7.1 — Quality does not degrade over 4+ hours

**Prediction:** Eval score on canonical tasks at hour 4 is within 10% of hour 1. Tool-call JSON validity rate at hour 4 is within 5% of hour 1.

**Decision rule:**
- Hour 4 eval within 10% of hour 1, validity within 5% → PASS.
- 10-20% degradation → monitor in production; consider periodic vLLM restart.
- > 20% degradation → investigate KV cache management, consider shorter vLLM session TTL.

### H-7.2 — No memory leaks over sustained run

**Prediction:** VRAM + system RAM usage on hour 4 matches hour 1 (±5%). Container count stable. No OOM-killer events.

**Decision rule:**
- Stable RAM, no OOM → PASS.
- Gradual VRAM creep → investigate vLLM KV cache release patterns.
- OOM events → fundamental leak; must fix before production.

### H-7.3 — Retry cascades don't pile up under fault conditions

**Prediction:** When a component (e.g., a worker's vLLM) is deliberately killed mid-session, Triumvirate detects within 30 sec, retries with exponential backoff, does not exceed 3 retries per task. System recovers without human intervention.

**Decision rule:**
- Detected within 30 sec, recovers within 5 min → PASS.
- Detected but recovery takes > 10 min → tune retry logic.
- Cascade retries / death spiral → critical bug, cannot ship.

### H-7.4 — Evidence pipeline handles sustained volume

**Prediction:** 4-hour run produces an evidence bundle that uploads cleanly to GCS without truncation or corruption. Supabase metric ingestion keeps up.

**Decision rule:**
- Bundle uploads intact, metrics land in Supabase → PASS.
- Partial upload / ingestion gaps → tune bundle chunking + upload streaming.

---

## Pre-run checklist

- [ ] All prior gates passed
- [ ] Evidence pipeline (Supabase sync + Obsidian note generation) working
- [ ] Billing budget alert threshold suitable for $200-400 spend on this gate
- [ ] Fault-injection scripts ready for sub-gate 7c
- [ ] Long-run monitoring (Grafana dashboard or equivalent) pointed at VM metrics

---

## Runbook — Sub-gate 7a (KV cache soak)

### Step 1 — Provision full stack (same as Gate 5, but 8-hour max-run-duration)

```bash
export PROJECT_ID="pantheon-validation-v1"
export ZONE="us-central1-a"
export RUN_ID="gate7a-soak-$(date +%Y%m%d-%H%M%S)"
export REGISTRY="us-central1-docker.pkg.dev/${PROJECT_ID}/pantheon-images"

# Provision using Gate 5's composition, but with --max-run-duration=480m
# See runbooks/gate-5-full-trinity.md Step 1 — substitute 120m → 480m (8 hrs)
```

### Step 2 — Launch continuous-dispatch harness

```bash
gcloud compute ssh pantheon-$RUN_ID-orch --zone=$ZONE --command="
  gsutil -m cp -r gs://pantheon-fixtures/agent-tasks-soak-pool/ /tmp/task-pool
  
  # Dispatch tasks continuously for 4 hours
  # Harness: picks random task from pool, dispatches, collects result, captures
  # hourly metric snapshot, uploads per-hour evidence slice to GCS
  docker run --rm --network host \
    -v /tmp/task-pool:/tasks:ro \
    -e TEST=h-7.1-kv-soak -e RUN_ID=$RUN_ID \
    $REGISTRY/pantheon-test-harness:main \
    --mode=continuous-dispatch \
    --triumvirate-url=http://localhost:7788 \
    --task-pool=/tasks \
    --duration-hours=4 \
    --dispatch-rate-per-hour=15 \
    --metric-snapshot-interval-min=15 \
    --output-dir=/tmp/evidence/$RUN_ID \
    --upload-to-gcs=gs://pantheon-evidence/gate-7a/$RUN_ID/ \
    --upload-interval-min=60
"
```

### Step 3 — Monitor progress (from laptop)

```bash
# Every 30 min, pull snapshot of current metrics
while true; do
  gsutil cat gs://pantheon-evidence/gate-7a/$RUN_ID/hourly-metrics-latest.json 2>/dev/null | jq .
  sleep 1800
done
```

### Step 4 — Evidence + self-destruct (automatic at 8-hour cap)

```bash
# Harness auto-uploads final bundle and kills all VMs
# Manual cleanup if needed:
for VM in zeus athena vulcan orch; do
  gcloud compute instances delete pantheon-$RUN_ID-$VM --zone=$ZONE --quiet 2>/dev/null &
done
wait
```

---

## Runbook — Sub-gate 7b (concurrent-load sustained)

Same VM composition as 7a. Different harness:

```bash
docker run --rm --network host \
  -v /tmp/task-pool:/tasks:ro \
  -e TEST=h-7.2-sustained-concurrent -e RUN_ID=$RUN_ID \
  $REGISTRY/pantheon-test-harness:main \
  --mode=sustained-concurrent \
  --triumvirate-url=http://localhost:7788 \
  --task-pool=/tasks \
  --concurrent-streams=8 \
  --duration-hours=4 \
  --metric-snapshot-interval-min=15 \
  --gpu-metrics-interval-sec=30 \
  --output-dir=/tmp/evidence/$RUN_ID
```

Captures: per-stream tok/s over time, VRAM utilization curves, tool-call latency distribution, any dropped requests.

---

## Runbook — Sub-gate 7c (fault injection)

This sub-gate is where Pantheon's retry + recovery logic is stressed.

### Fault scenarios (run in sequence, 30 min apart):

**Fault 1 — Vulcan vLLM kill**
```bash
# At T+30 min, on Vulcan VM:
docker kill vllm-vulcan

# Observe: Triumvirate should detect unhealthy endpoint within 30s,
# route fast-fix requests to fallback (Athena) or skip Vulcan entirely,
# NOT cascade-retry the dead endpoint indefinitely.
# At T+45 min, restart Vulcan:
docker start vllm-vulcan

# Observe: Triumvirate detects recovery, resumes normal routing.
```

**Fault 2 — Network saturation**
```bash
# Use iperf3 or stress-ng --netio to saturate inter-VM bandwidth
# Observe: tool-call timeouts handled gracefully, requests queued or rejected
# appropriately
```

**Fault 3 — Corrupted tool-call response**
```bash
# Inject malformed JSON via mock endpoint that intercepts
# Observe: Triumvirate's schema validator catches, triggers retry,
# does not crash daemon.
```

**Fault 4 — PD disk saturation**
```bash
# Fill model disk with garbage to trigger inference failure
# Observe: graceful degradation, not silent corruption.
```

For each fault, capture: detection time, recovery time, whether human intervention was needed, whether subsequent tasks completed normally.

---

## Evidence bundle structure for Gate 7

Gate 7 evidence is structured differently — time-series heavy, large volume:

```
gs://pantheon-evidence/gate-7a/$RUN_ID/
├── manifest.json
├── hourly-metrics/
│   ├── hour-01.json
│   ├── hour-02.json
│   ├── hour-03.json
│   └── hour-04.json
├── gpu-timeseries.csv       ← 30-sec samples over 4 hrs
├── task-outcomes/           ← one JSON per dispatched task
│   ├── task-0001.json
│   ├── task-0002.json
│   └── ...
├── vllm-logs/
│   ├── zeus.log
│   ├── athena.log
│   └── vulcan.log
├── triumvirate.log
├── fault-log.json           ← (for sub-gate 7c only)
└── summary.md
```

---

## Cost accounting

| Sub-gate | Duration | Hourly | Session |
|---|---|---|---|
| 7a — KV cache soak | 4 hrs | ~$42/hr | ~$170 |
| 7b — concurrent sustained | 4 hrs | ~$42/hr | ~$170 |
| 7c — fault injection | 2-4 hrs | ~$42/hr | ~$85-170 |
| **Total** | | | **~$425-510** |

Budget roughly $500 for Gate 7 end-to-end. Run once pre-customer-ship, then quarterly to detect drift.

**This is the single most expensive gate.** Its value: catching the issues that would embarrass Pantheon in a 4-day customer pilot.

---

## Decision rule application

**Ready to ship sovereign production deployment if:**
- H-7.1, H-7.2, H-7.3, H-7.4 all PASS
- No unexplained anomalies in time-series metrics
- Sub-gate 7c shows clean recovery from all fault scenarios

**Hold before shipping if:**
- Any hypothesis fails
- Time-series reveals gradual degradation (even if PASS threshold technically met)
- Fault recovery takes > 10 min for any scenario

---

## Cadence

**Pre-ship (one-time):** full Gate 7 required before first paying sovereign customer.

**Quarterly:** run sub-gate 7a only (4 hrs, ~$170) to detect drift as components upgrade.

**Pre-deal:** run sub-gate 7c (fault injection) before any enterprise pilot to validate current state handles planned-failure scenarios.

---

## What comes after

Gate 7 PASS = Pantheon is **production-shipping ready** at whatever tier has been validated (Desk, Closet, or Rack depending on which hardware composition you soaked).

Gate 7 is the terminal gate. Everything downstream is OPERATIONAL RUN — real customer engagements, real revenue, real compounding moat.

---

## Final note

If you've executed Gates 0 through 7 cleanly, you have:

1. Empirical validation across 7 hardware tiers + architectural patterns
2. Evidence bundles for every gate, queryable forever
3. Obsidian vault populated with 7+ run notes + 10+ lessons + 5-10 decisions
4. Pre-committed decision rules applied with evidence support
5. A knowledge graph that every future Pantheon run can consult
6. Proof of sovereign capability (Gate 6)
7. Proof of operational stability (Gate 7)
8. A reproducible test harness that can re-validate any component after upgrades

**Total cumulative spend across all 7 gates: ~$500-700.** That's the price of building Pantheon on GCP with full empirical confidence.

Then — when revenue triggers fire — you buy the hardware that the data says to buy. No vibes. No motivated reasoning. Just evidence.
