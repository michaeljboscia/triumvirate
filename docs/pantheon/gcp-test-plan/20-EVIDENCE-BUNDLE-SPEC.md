# Pantheon Evidence Bundle Specification

**Status:** canonical
**Applies to:** every gate run, preflight smoke test, any Pantheon GCP burn
**Storage root:** `gs://pantheon-evidence/{gate_id}/{run_id}/`

An **evidence bundle** is the durable, immutable, machine-readable artifact every Pantheon run MUST produce. No run is considered complete until its bundle lands in GCS. This spec defines the format, so any tool (harness, Obsidian sync, Supabase extractor, Pythia embedder, dashboard) can consume bundles consistently.

---

## Design goals

1. **Immutable.** Once written, never modified. New runs produce new bundles; corrections happen in downstream layers.
2. **Self-describing.** Manifest + schema version lets future tooling consume old bundles.
3. **Tool-agnostic.** Plain JSON + markdown + CSV + logs. No proprietary format.
4. **Queryable structurally.** Metrics in JSON with a stable schema → Supabase extraction is trivial.
5. **Queryable semantically.** Summary in markdown → Pythia embedding → semantic search forever.
6. **Human-readable.** You can `gsutil cat` any bundle and understand the run.
7. **Cheap to store.** Total size per run target: < 100MB.

---

## Bundle directory structure

```
gs://pantheon-evidence/gate-{N}/{run_id}/
├── manifest.json                  ← REQUIRED — metadata, hypotheses, verdicts
├── summary.md                     ← REQUIRED — human-readable narrative
├── cost-report.json               ← REQUIRED — GCP spend attribution
├── obsidian-note.md               ← REQUIRED — vault-ready markdown
├── metrics/
│   ├── h-{N}-{test_id}.json       ← per-hypothesis metrics, one file each
│   └── nvidia-smi.csv             ← GPU time-series samples
├── logs/
│   ├── triumvirate.log
│   ├── vllm-{role}.log            ← one per vLLM container
│   ├── docker-compose.log         ← if compose used
│   └── startup-script.log
├── artifacts/
│   ├── generated-code/            ← outputs from agent tasks
│   ├── evaluations/               ← eval scores per task
│   └── checkpoints/               ← LoRA adapters / training state (when applicable)
└── raw/                           ← raw tcpdump / strace / etc. (gate-specific)
```

---

## Required file schemas

### `manifest.json`

Written at run start, finalized at run end. Must conform to schema version.

```json
{
  "schema_version": "1.0",
  "run_id": "gate2-dual-l4-20260418-153022-a7f3",
  "gate": 2,
  "gate_name": "Dual L4 (3090 Pair Proxy)",
  "experimenter": "mike-boscia",
  "triumvirate_version": "0.3.2",
  "git_commit": "abc1234de",
  "started_at": "2026-04-18T15:30:22Z",
  "ended_at": "2026-04-18T17:28:44Z",
  "duration_sec": 7102,
  "gcp_project": "pantheon-validation-v1",
  "gcp_region": "us-central1",
  "gcp_zone": "us-central1-a",
  "gcp_machine_types": ["g2-standard-24"],
  "gcp_accelerators": ["2x nvidia-l4"],
  "gcp_provisioning_model": "SPOT",
  "models_used": [
    {"role": "zeus", "model": "qwen2.5-72b-instruct-awq", "quantization": "awq_marlin"},
    {"role": "coder", "model": "qwen2.5-coder-32b-instruct-awq", "quantization": "awq_marlin"}
  ],
  "hypotheses_tested": ["H-2.1", "H-2.2", "H-2.3"],
  "verdicts": {
    "H-2.1": "PASS",
    "H-2.2": "PASS",
    "H-2.3": "INCONCLUSIVE"
  },
  "prior_runs_referenced": ["gate1-single-l4-20260417-..."],
  "decision_rules_applied": ["Decision-1"],
  "decision_rule_outcomes": {
    "Decision-1": {
      "verdict": "buy 2x 3090 NVLink",
      "confidence": 0.85,
      "supporting_hypotheses": ["H-2.1", "H-2.2"]
    }
  },
  "total_cost_usd": 0.86,
  "evidence_bundle_size_mb": 14.2,
  "links": {
    "summary": "gs://pantheon-evidence/gate-2/{run_id}/summary.md",
    "cost_report": "gs://pantheon-evidence/gate-2/{run_id}/cost-report.json",
    "obsidian_note": "gs://pantheon-evidence/gate-2/{run_id}/obsidian-note.md",
    "prior_run_chain": ["gs://pantheon-evidence/gate-1/..."]
  }
}
```

**Fields MUST include:** schema_version, run_id, gate, started_at, ended_at, gcp_machine_types, hypotheses_tested, verdicts, total_cost_usd.

### `summary.md`

Human-readable, markdown, auto-generated from manifest + metrics + logs. Template defines the shape.

```markdown
# Run {run_id} — {gate_name}

**Date:** {started_at}  
**Duration:** {duration_sec}s ({duration_hm})  
**Cost:** ${total_cost_usd}  
**Verdict:** {overall_verdict}

## Setup
- Machine: {gcp_machine_types}
- Models: {models_used}
- Triumvirate version: {triumvirate_version} ({git_commit})

## Hypotheses tested

### {hypothesis_id} — {hypothesis_description}
**Prediction:** {prediction_text}
**Result:** {verdict}

| Metric | Target | Actual | Verdict |
|---|---|---|---|
{metric_rows}

## Observations
{auto_extracted_observations_from_logs_and_metrics}

## Anomalies
{any_metric_outliers_or_error_events}

## Decision rules applied
{decision_rule_outcomes}

## Next actions
- [ ] {suggested_followups}

## Links
- Evidence bundle: `gs://pantheon-evidence/gate-{N}/{run_id}/`
- Prior runs: {links}
```

### `cost-report.json`

```json
{
  "schema_version": "1.0",
  "run_id": "gate2-dual-l4-20260418-153022-a7f3",
  "billing_account": "XXXXXX-XXXXXX-XXXXXX",
  "line_items": [
    {
      "service": "Compute Engine",
      "sku": "Spot Preemptible G2 Instance Core running in Americas",
      "usage_amount": 0.5833,
      "usage_unit": "hours",
      "cost_usd": 0.23
    },
    {
      "service": "Compute Engine",
      "sku": "Spot Preemptible NVIDIA L4 GPU running in Americas",
      "usage_amount": 1.1666,
      "usage_unit": "hours",
      "cost_usd": 0.48
    },
    {
      "service": "Compute Engine",
      "sku": "Spot Preemptible SSD backed PD Capacity",
      "usage_amount": 0.58,
      "usage_unit": "GB-hours",
      "cost_usd": 0.04
    },
    {
      "service": "Cloud Storage",
      "sku": "Standard Storage US Multi-region",
      "usage_amount": 0.014,
      "usage_unit": "GB-months",
      "cost_usd": 0.0003
    }
  ],
  "total_cost_usd": 0.86,
  "budgeted_cost_usd": 0.90,
  "within_budget": true,
  "attribution_method": "label-based query on billing export"
}
```

### `obsidian-note.md`

Ready-to-drop-into-vault markdown with YAML frontmatter that Dataview can query.

```markdown
---
type: pantheon-run
run_id: gate2-dual-l4-20260418-153022-a7f3
date: 2026-04-18
gate: 2
gate_name: "Dual L4 (3090 Pair Proxy)"
gcp_machine: g2-standard-24
region: us-central1
duration_min: 118
cost_usd: 0.86
models:
  - qwen2.5-72b-instruct-awq
  - qwen2.5-coder-32b-instruct-awq
hypotheses_tested:
  - H-2.1
  - H-2.2
  - H-2.3
verdicts:
  H-2.1: PASS
  H-2.2: PASS
  H-2.3: INCONCLUSIVE
overall_verdict: PASS
tags:
  - pantheon-run
  - gate-2
  - dual-l4
  - rtx-3090-proxy
  - concurrent-serving
  - quantization-awq
significance: 3
---

# Run {run_id} — Gate 2 Dual L4

**Verdict:** {overall_verdict}

## Summary
{summary_text_auto_generated}

## Hypotheses

### [[H-2.1]] — 70B-Q4 local inference is usable
**Result:** {verdict_with_metrics}

### [[H-2.2]] — Concurrent multi-model hosting works
**Result:** {verdict_with_metrics}

### [[H-2.3]] — 32B LoRA training is feasible
**Result:** {verdict_with_metrics}

## Observations
{observations_awaiting_human_annotation}

## Links
- Evidence bundle: `gs://pantheon-evidence/gate-2/{run_id}/`
- [[decision-1-3090-purchase]]
- Prior run: [[gate1-single-l4-...]]

## Mike's notes
<!-- Add qualitative observations here after reviewing bundle -->
```

### `metrics/h-{N}-{test_id}.json`

One file per hypothesis test, consistent schema.

```json
{
  "schema_version": "1.0",
  "test_id": "h-2.1-single-stream",
  "run_id": "{run_id}",
  "hypothesis": "H-2.1",
  "started_at": "2026-04-18T15:42:11Z",
  "ended_at": "2026-04-18T15:47:11Z",
  "duration_sec": 300,
  "harness_mode": "throughput-sustained",
  "endpoint": "http://localhost:8000/v1",
  "model": "qwen2.5-72b-instruct-awq",
  "concurrency": 1,
  "input": {
    "prompt_set": "standard-agent-prompts-v1",
    "num_prompts": 42,
    "total_input_tokens": 21000
  },
  "output": {
    "total_output_tokens": 12600,
    "requests_completed": 42,
    "completion_errors": 0
  },
  "metrics": {
    "tokens_per_second_per_stream_median": 12.4,
    "tokens_per_second_per_stream_p95": 10.1,
    "tokens_per_second_per_stream_p99": 8.6,
    "time_to_first_token_ms_median": 320,
    "time_to_first_token_ms_p95": 580,
    "wall_clock_per_completion_sec_median": 24.3,
    "json_schema_validity_rate": 0.95,
    "tool_call_success_rate": 0.92,
    "retry_count_mean": 0.12
  },
  "targets": {
    "tokens_per_second_per_stream_min": 10,
    "json_schema_validity_rate_min": 0.90
  },
  "verdict": "PASS",
  "decision_rule_applied": "H-2.1 → ≥10 tok/s single → PASS confirms 70B-local usable"
}
```

### `metrics/nvidia-smi.csv`

Time-series GPU utilization. Standard nvidia-smi CSV format.

```csv
timestamp,index,utilization.gpu [%],memory.used [MiB],temperature.gpu,power.draw [W]
2026-04-18T15:30:22.000000,0,0,320,42,38.50
2026-04-18T15:30:52.000000,0,87,18432,64,220.85
2026-04-18T15:31:22.000000,0,91,18688,68,238.14
...
```

Sample interval: 30 sec default, configurable per gate.

---

## Lifecycle — when each file is written

```
T+0   Provisioning
      └─ manifest.json created with status="running"
      └─ Start tcpdump/strace if required by gate

T+0-2 min   VM boot + stack startup
      └─ logs/startup-script.log appended in real time

T+2-N min   Test execution
      └─ Per hypothesis, metrics/h-{N}-*.json written at test end
      └─ Per container, logs/vllm-{role}.log written continuously

T+N min   Finalization (runner script)
      └─ nvidia-smi --query-gpu=... captured to metrics/nvidia-smi.csv
      └─ manifest.json updated with verdicts + ended_at
      └─ summary.md auto-generated from template
      └─ cost-report.json generated by cost-tracker.py
      └─ obsidian-note.md rendered from template
      └─ ALL files uploaded to gs://pantheon-evidence/gate-{N}/{run_id}/

T+N min   Self-destruct
      └─ VM deleted via trap or --max-run-duration
```

---

## Downstream consumers

Once bundle lands in GCS, six automations trigger:

### 1. Supabase extraction (within 60 sec of bundle landing)

Cloud Function watches `gs://pantheon-evidence/` for new objects. On `manifest.json` write:

- Row inserted into `pantheon_runs` (core metadata)
- Per-hypothesis rows into `run_hypotheses`
- Per-metric rows from `metrics/*.json` into `run_metrics`
- Cost line items from `cost-report.json` into `run_costs`

### 2. Pythia semantic embedding (within 5 min)

- `summary.md` + observations extracted and embedded via BGE-Large
- Inserted into Pythia corpus with tags from manifest (gate, models, verdict)
- Now queryable: `lcs_investigate "gate 2 dual L4 concurrent serving"` returns this run

### 3. Obsidian vault sync (within 5 min)

- `obsidian-note.md` copied to `~/Documents/pantheon-vault/runs/{run_id}.md`
- Git auto-commits the new note

### 4. Dashboard refresh (on next hour)

- Grafana / Streamlit reads Supabase, regenerates week/month trend charts
- Cost-per-insight KPI recomputed

### 5. Hypothesis tracker update (manual gate, weekly review)

- Mike reviews the run, updates `open-hypotheses.md` confidence tracker
- Candidate observations promoted to `lessons/candidates.md` if interesting

### 6. Alert on failure

- If `verdicts` contains any FAIL, PubSub alert fires
- Slack/SMS notification to Mike

---

## Storage economics

| Gate | Bundle size (typical) | Cost per bundle |
|---|---|---|
| Gate 0 | ~2 MB | $0.00004 |
| Gate 1 | ~10 MB | $0.0002 |
| Gate 2 | ~15 MB | $0.0003 |
| Gate 3 | ~20 MB | $0.0004 |
| Gate 4 | ~35 MB | $0.0007 |
| Gate 5 | ~80 MB | $0.0016 |
| Gate 6 | ~10 MB | $0.0002 |
| Gate 7 (4hr soak) | ~500 MB | $0.01 |

All in standard storage at $0.020/GB/month.

**100 runs/year = ~5GB of bundles = $1.20/year in storage.** Effectively free forever.

---

## Retention policy

- **All bundles retained forever.** They're the historical record.
- **Promote interesting bundles to nearline** (via Lifecycle policy) after 90 days to save storage cost. Bundles rarely accessed after 90 days become $0.01/GB/mo instead of $0.020.
- **NEVER delete bundles.** They're immutable evidence and represent paid compute.

---

## Schema versioning

When this spec changes:

1. Increment `schema_version` field (currently `1.0`)
2. Document breaking changes in this file's Changelog section
3. Downstream consumers must handle both old + new schemas (no deletion of old)
4. Migration scripts (if needed) live in `harness/migrations/`

**Changelog:**
- `1.0` — 2026-04-18 — initial spec

---

## What this spec enables

Because every gate run produces a bundle with THIS format, you get for free:

1. **Semantic search** across every run ever done (Pythia)
2. **Structured queries** across every metric ever collected (Supabase)
3. **Human-readable archive** (Obsidian vault)
4. **Reproducibility** — anyone with a bundle can see exactly what config produced what result
5. **Cost accountability** — every dollar traced to a specific run and hypothesis
6. **Decision audit trail** — "why did we buy the RTX Pro 6000?" → evidence bundle from Gate 3 with verdict PASS and decision rule applied
7. **Knowledge moat** — your test history IS the moat. Competitors would have to re-run every experiment to catch up.

**Evidence bundles are the primary artifact of Pantheon validation. Guard them like production data.**
