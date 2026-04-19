# Pantheon Decision Rules — Pre-Committed

**Status:** canonical · commit BEFORE running tests, apply AFTER evidence lands
**Purpose:** Capture every decision the Pantheon program faces as a pre-committed rule with explicit evidence-based triggers. Prevents post-hoc rationalization and motivated reasoning.

---

## Why pre-committed rules

Every decision in this doc is written BEFORE the evidence arrives. When the evidence lands, the rule is applied mechanically — not debated. If the evidence is ambiguous, the rule's fallback language specifies what happens.

**If a rule feels wrong AFTER evidence arrives, that's the signal that motivated reasoning is active.** Change the rule with a dated amendment, not silent reinterpretation.

---

## How each rule is structured

```
Decision N — {name}
  Trigger:       when this decision is actually forced
  Evidence source: which gate(s) feed the decision
  Rule:          the if-then specification
  Fallback:      what to do if evidence is INCONCLUSIVE
  Amendment log: dated changes (never deletions)
```

---

## Decision 1 — Single 3090 vs 2× 3090 NVLink purchase

**Trigger:** Funds available (estimated 3-6 weeks from 2026-04-18). Gates 1 + 2 evidence in GCS.

**Evidence source:** Gate 1 (`runbooks/gate-1-single-l4.md`) + Gate 2 (`runbooks/gate-2-dual-l4.md`) + 4-week friction log.

### Rule

**Buy 2× 3090 NVLink** (~$3500-4500 total) if ALL:
- Gate 2 H-2.1 single-stream ≥ 10 tok/s on Qwen 72B-Q4
- Gate 2 H-2.1 4-way batched ≥ 5 tok/s per stream
- Gate 2 H-2.2 concurrent hosting contention factors both < 2.0
- Gate 2 H-2.3 32B LoRA completes in ≤ 3 hrs without OOM
- 4-week friction log shows ≥ 5 events/week where specifically the 48GB tier mattered

**Buy single 3090** (~$2500 total) if:
- Gate 1 H-1.1 through H-1.4 all PASS (24GB sufficient for daily work)
- Gate 2 H-2.1 70B-local is slow or underwhelming (< 5 tok/s)
- Friction log shows < 5/week 48GB events
- Your LoRA targets stay at 7-14B base models

**Skip local GPU entirely** if:
- Friction log shows < 3 events/week where any local GPU would help
- Pre-bake tooling makes GCP spin-up feel frictionless (< 90s perception)
- Mac Studio (Phase 2) arrival is imminent (<4 weeks) and covers remaining needs

### Fallback (INCONCLUSIVE evidence)

If Gate 2 results are mixed (some hypotheses PASS, others FAIL):
- Re-run Gate 2 once to rule out flaky measurement
- If still mixed, default to **single 3090** (more reversible; fewer regret modes)

### Amendment log
- 2026-04-18 — initial commitment

---

## Decision 2 — RTX Pro 6000 Blackwell purchase

**Trigger:** Any one of these signals fires:
- GCP G4 usage exceeds $1000/mo for 2 consecutive months on consistent workload
- Signed customer engagement with line-item hardware allocation in contract
- Training workload bottlenecked on 3090 pair for 3+ consecutive weeks
- Enterprise / sovereign prospect requires 70B-at-production-speed on-premise

**Evidence source:** Gate 3 (`runbooks/gate-3-rtx-pro-6000.md`) + Supabase monthly GCP spend + Obsidian friction log.

### Rule

**Buy 1× RTX Pro 6000 Blackwell workstation** (~$13-15K) if ALL:
- Gate 3 H-3.1 72B single-stream ≥ 50 tok/s (production-viable Zeus)
- Gate 3 H-3.2 three-model concurrent hosting works (contention factors < 1.5×)
- Gate 3 H-3.3 32B LoRA single-card completes in ≤ 2 hrs
- Gate 3 H-3.4 canonical 8-task agent swarm hits ≥ 6/8 pass
- AND at least one trigger above has fired

**Defer purchase** if:
- No trigger has fired (OPEX still cheaper)
- Gate 3 results underwhelm (pending investigation)

### Fallback (INCONCLUSIVE evidence)

If Gate 3 partially passes, re-run once after addressing known issues (e.g., vLLM version, driver mismatch). If still partial, defer purchase until next trigger fire + fresh Gate 3.

### Amendment log
- 2026-04-18 — initial commitment

---

## Decision 3 — 2nd RTX Pro 6000 / scale-out

**Trigger:** After RTX Pro 6000 in operation for ≥ 60 days AND any of:
- Training blocks production serving for 3+ consecutive weeks
- Multiple paying customers cause scheduling contention
- Enterprise deal funds the purchase as contract line-item
- GPU utilization on first card > 80% sustained for 4+ weeks

**Evidence source:** Operational logs post-hardware-purchase (Supabase `gpu_utilization` time-series).

### Rule

**Buy 2nd RTX Pro 6000** (+$10-13K) if ALL:
- At least one trigger above has fired
- Supabase data shows first card sustained > 70% utilization for 30+ days
- Workload mix includes both training + serving where scheduling conflicts are measurable

**Sub-rule: Same chassis (Phase 3a) vs separate chassis (Phase 3b)**
- Phase 3a (single chassis, ~$10K): if training iteration speed is the primary bottleneck
- Phase 3b (separate chassis, ~$13K): if redundancy + parallel serving is the primary need

### Fallback

If utilization is < 70%, the card isn't earning a sibling. Keep using GCP burst for peak loads.

### Amendment log
- 2026-04-18 — initial commitment

---

## Decision 4 — Gate 0 plumbing PASS/FAIL protocol

**Trigger:** Every Gate 0 run.

**Evidence source:** Gate 0 bundle metrics.

### Rule

**PASS** → proceed to Gate 1 when ready. No further Gate 0 runs required unless Triumvirate code changes materially.

**FAIL** → debug the specific failing component before spending GPU dollars:
- NATS health check fails → investigate NATS container, compose networking
- Mock vLLM unhealthy → check test-harness image mock-vllm mode
- Triumvirate startup fails → review logs, Rust error messages, config validation
- End-to-end 5-task dispatch fails → trace with RUST_LOG=debug, identify where tasks stall

Do NOT proceed to any GPU gate until Gate 0 passes cleanly.

---

## Decision 5 — Pantheon core thesis validation (Gate 4)

**Trigger:** Gate 4 run completion.

**Evidence source:** Gate 4 bundle (`runbooks/gate-4-athena-swarm.md`).

### Rule

**Thesis validated** if ALL:
- H-4.1: 4/4 worktrees created, ≥ 3/4 merge cleanly
- H-4.2: ≥ 80% of generated code passes basic validation (imports, syntax, signatures)
- H-4.3: Median task wall-clock ≤ 15 min, max ≤ 30 min
- H-4.4: Winning serving mode (TP=4 vs 4-process) identified with clear evidence

**Thesis PARTIALLY validated** if 2-3 of the above PASS but one fails clearly:
- Investigate the failure, patch, re-run that hypothesis
- Do NOT proceed to Gate 5 until H-4.1 specifically passes (worktree + merge is the core thesis)

**Thesis FALSIFIED** if H-4.1 fails (< 3/4 merge) OR H-4.2 fails (< 50% code validity):
- Architectural investigation required BEFORE further investment
- Most likely culprits: Pythia retrieval misaligned, worktree isolation leaky, prompt engineering inadequate
- Document findings in `lessons/pantheon-thesis-failure.md`, propose redesign

### Amendment log
- 2026-04-18 — initial commitment

---

## Decision 6 — Pantheon Rack tier purchase (Gate 5 gates this)

**Trigger:** Enterprise customer engagement signed OR self-directed decision to invest $150-500K.

**Evidence source:** Gate 5 bundle (`runbooks/gate-5-full-trinity.md`) + business pipeline data.

### Rule

**Purchase enterprise hardware** (used 8× A100 workstation OR DGX H100 OR equivalent, $80-500K) if ALL:
- Gate 5 all five hypotheses PASS
- Revenue identified that justifies the CAPEX (either single engagement or pipeline)
- Customer-funded option is NOT available (otherwise prefer customer-funded purchase)

**Do NOT self-fund Pantheon Rack purchase** if:
- No paying customer engagement requires it
- Cloud burst (GCP a2-ultragpu-8g on-demand) can serve the workload at lower cost
- You haven't hit at least $500K/year in revenue on the business

### Fallback

If Gate 5 partially passes but no customer is waiting, defer hardware. Continue running Gate 5 quarterly on GCP to track readiness.

---

## Decision 7 — Sovereign customer ship-readiness

**Trigger:** Any sovereign customer demo scheduled OR sovereign-tier contract negotiation.

**Evidence source:** Gate 6 bundle (`runbooks/gate-6-airgap-sanity.md`) — MUST be within last 30 days.

### Rule

**Ship sovereign demo** if ALL:
- Gate 6 H-6.1 zero outbound traffic (≤ 5 incidental packets, all to PGA endpoints)
- Gate 6 H-6.2 full 4-task canonical swarm completes air-gapped
- Gate 6 H-6.3 evidence bundle lands via PGA
- Bundle timestamp is within 30 days of the demo

**Do NOT ship sovereign demo** if:
- Most recent Gate 6 run is > 30 days old (claim may have drifted with component updates)
- Any H-6 hypothesis failed in last Gate 6 run
- Pantheon has upgraded any component since last Gate 6 (must re-run)

### Amendment log
- 2026-04-18 — initial commitment

---

## Decision 8 — Production shipping readiness (Gate 7)

**Trigger:** First paying sovereign or enterprise customer engagement.

**Evidence source:** Gate 7 bundle (`runbooks/gate-7-soak-stress.md`) — sub-gates 7a, 7b, 7c all complete within last 60 days.

### Rule

**Ship to production customer** if ALL:
- Gate 7a (4hr KV soak) PASS: quality degradation < 10% hour 1 to hour 4, validity delta < 5%
- Gate 7b (4hr sustained concurrent) PASS: no memory leaks, no OOM
- Gate 7c (fault injection) PASS: recovery from each fault scenario within 10 min without human intervention

**Hold before shipping** if:
- Any sub-gate failed or not yet run
- Time-series reveals gradual degradation even if within thresholds
- Fault recovery took > 10 min for any scenario

### Amendment log
- 2026-04-18 — initial commitment

---

## Decision 9 — Mac Studio purchase (Phase 2)

**Trigger:** WWDC 2026 Mac Studio M5 Ultra announcement + first paying engagement closed.

**Evidence source:** Apple announcement (memory tiers, pricing) + business cash flow + friction log entries specifically citing Mac-native workflows.

### Rule

**Buy Mac Studio 256GB** (~$8K) if ALL:
- Apple ships 256GB or larger Mac Studio M5 Ultra at WWDC 2026
- First paying engagement invoiced + paid (revenue buffer > $30K)
- Friction log shows ≥ 3 events/week where Metal/MLX/Whisper/macOS ecosystem was the blocker
- Current workflow shows ≥ 20% of Mike's time in macOS-native tools

**Buy Mac Studio 512GB** (~$12K) instead of 256GB only if:
- Specific customer tier (Sovereign with 405B-local requirement) justifies the +$4K
- OR Apple's 256GB tier is unavailable and 128GB is the max below 512GB

**Skip Mac Studio entirely** if:
- RTX Pro 6000 already owned AND covers the specialist-fleet + Metal-equivalent needs (unlikely)
- Revenue hasn't landed OR Mike's workflow doesn't require macOS ecosystem
- Cash flow priorities other (runway preservation > hardware acquisition)

### Fallback (Apple WWDC doesn't ship 512GB)

If only 256GB is available at WWDC: default to 256GB and defer 512GB question to next Mac Studio cycle (~2-3 years).

### Amendment log
- 2026-04-18 — initial commitment

---

## Decision 10 — OPEX vs CAPEX threshold crossover

**Trigger:** Monthly, reviewed as part of financial snapshot.

**Evidence source:** Supabase `gcp_monthly_spend` view + `gpu_utilization_hours_per_month` KPI.

### Rule

**Convert OPEX to CAPEX** (buy local hardware replacing a specific GCP workload) if ALL:
- Monthly GCP spend on that workload class > $1000 for 2 consecutive months
- Workload is consistent (not a one-time burst)
- Usage hours exceed break-even for the specific hardware class:
  - RTX Pro 6000 class: > 150 hrs/mo
  - A100 80GB class: > 230 hrs/mo
  - H100 class: > 360 hrs/mo

**Stay OPEX** if:
- Any of the above conditions not met
- Workload is bursty / unpredictable
- Cash flow priorities elsewhere

### Fallback

Review monthly. If borderline (usage 100-150 hrs/mo for RTX Pro 6000 class), wait one more month before deciding.

---

## Rule application log

Every decision applied produces a log entry in the evidence bundle's `decision-rule-outcomes.json` + the Obsidian vault's `decisions/` folder.

```json
{
  "decision_id": "Decision-1",
  "applied_at": "2026-05-15T10:00:00Z",
  "evidence_run_ids": ["gate1-single-l4-...", "gate2-dual-l4-..."],
  "rule_path": "ALL H-2.1, H-2.2, H-2.3 PASS + friction log ≥5/week",
  "observed_state": {
    "h-2.1": "PASS",
    "h-2.2": "PASS",
    "h-2.3": "PASS",
    "friction_events_per_week_avg": 7
  },
  "verdict": "buy 2x 3090 NVLink",
  "confidence": 0.90,
  "next_actions": ["source 2x used 3090", "spec workstation build", "budget $4000"]
}
```

---

## Amendment protocol

Rules can be amended but NEVER silently changed. If a rule feels wrong after evidence arrives:

1. Write a dated amendment in the rule's Amendment Log
2. Explain the new understanding
3. Apply only to future evidence (old evidence under old rule)

**Never**: backdate an amendment or apply a newer rule retroactively to explain a decision already made.

---

## What this document enables

1. **Evidence-based hardware procurement.** Every purchase is supported by a specific gate run + rule application.
2. **Motivated-reasoning detection.** When a rule FEELS wrong post-evidence, that's the diagnostic. Amend with awareness.
3. **Auditable decision trail.** Every purchase can be traced back to the specific evidence + rule that triggered it. Useful for customer trust, investor pitches, self-review.
4. **Reduced decision fatigue.** Most decisions apply mechanically from rules. Mental bandwidth preserved for building.
5. **Clean business defensibility.** "Why did you spend $15K on the RTX Pro 6000?" → "Rule 2 triggered when GCP spend exceeded $1000/mo for 2 months + Gate 3 PASS evidence. Here's the bundle."

**This document is the single source of truth for every Pantheon hardware + shipping decision. Amendments dated, never silent.**
