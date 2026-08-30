# Pantheon Risk Registry

**Purpose:** Track architectural, commercial, legal, and strategic landmines we've conceived of on Pantheon's path but don't have enough real implementation or market experience to specify further.

**Format:** Living document. Entries added when concerns surface. Reviewed per-quarter or when an entry becomes acute. Not a spec — does NOT commit Pantheon to design decisions. REQ anchors live in `pantheon-gates-rust-port.md`; this file captures what we're AWARE of but explicitly NOT yet specifying.

**Review cadence:** Informal — revisit when (a) a scheduled quarterly review, (b) a commercial engagement forces attention, (c) any entry's severity changes, (d) a new landmine gets surfaced mid-session.

---

## Empirical-deferral posture (established end of Round 4, 2026-04-23)

Pantheon's spec envelope for detailed architectural commitment is **Gate 0 through Gate 2.C** — the gates that can be fully exercised inside the current hardware envelope (Mac + homebox). Gate 3 (Vulcan-1 sovereign) and everything downstream require hardware and market conditions we have not yet observed.

**Rule going forward:**
- **No new REQ anchors** for Gate 3+ concerns until empirical data from Gate 0-2.C runs exists.
- **Existing Gate 3+ REQ anchors stay filed as audit trail** — they capture architectural thinking at a point in time — but **implementation-level specificity for them is deferred** until we have cluster data that informs the shape.
- **Specification effort and risk-registry awareness of Gate 3+ concerns continues** — we keep thinking about them, capturing landmines, noting prior art — but we stop *baking in* design decisions for conditions we cannot test.

**REQ anchors in the "audit-trail-only / implementation-deferred" category** (filed but awaiting empirical input before further specification):
- REQ-088 (Gate 3 Vulcan-1 sovereign-path validation)
- REQ-092 (OpenTelemetry + AI observability for local-inference gates)
- REQ-095 partial — "N=5+ commercial target" and "specialist-composability" aspects; the "N≥3 floor + panel-parametric schema at Gate 1" parts remain active
- REQ-096 (benchmark-import pool extension)
- REQ-097 (Pantheon Federal deployment profile)

**Rationale (user-articulated, Round 4 close):** "we're going to be absolutely guessing if we try baking things in too far beyond what we can test inside the 4 walls of our house right now — and the things we learn going up to Gate 2.C will inform the rest of the gates, REQs, and design."

**Active spec envelope through next implementation cycle:** Gate 0 (authored), Gate 1 (Section 1 authored, 2-5 pending), Gate 1b (anchored, authoring queued), Gate 1.5 (anchored, authoring queued), Gate 2 (anchored, authoring queued), Gate 2.C (anchored, authoring queued). All runnable at current hardware scale. All produce empirical data that informs Gate 3+ specification when that work begins.

**Commercial positioning stays at current depth.** Fort Liberty / Old Iron / Wiza briefings may cite Pantheon Federal at commercial-positioning depth — the product tier has a name and a committed posture — without that requiring deeper architectural specification than REQ-097 already provides. The pitch materials are not spec; spec is not pitch materials.

---

**Severity scale:**
- **HIGH** — blocks a major capability or engagement if not addressed
- **MEDIUM** — likely to cause friction; workable for a while
- **LOW** — aware of it; no immediate pressure

**Disposition:**
- **WATCHING** — surfaced, tracked, no action yet
- **AWAITING TRIGGER** — waiting for a specific condition (hardware, customer, market event) to make it actionable
- **CANDIDATE FOR REQ** — close to ready for architectural REQ anchor
- **ACTIVE** — currently being worked
- **RETIRED** — no longer applicable (with reason)

---

## Architectural landmines (REQ-anchor candidates, deferred)

### R-001 — Specialist fine-tune provenance as mandatory audit artifact
**Category:** Architectural / Federal compliance
**Severity:** HIGH (Federal engagements cannot ship without this)
**Disposition:** AWAITING TRIGGER (first Federal engagement or Vulcan-1 operational)
**Description:** Every locally-trained specialist model needs full training-corpus manifest, weight digest, training environment fingerprint, and retraining history preserved as durable evidence alongside the evidence bundle. Partially flagged in REQ-097 commitment (5); needs its own REQ once training starts.
**Related REQs:** REQ-057 (environment fingerprint), REQ-072 (per-hypothesis baseline), REQ-097 (Pantheon Federal)
**Last surfaced:** 2026-04-23 (Round 4 landmine inventory)

### R-002 — Specialist lifecycle management
**Category:** Architectural / Operations
**Severity:** MEDIUM
**Disposition:** WATCHING
**Description:** A Python-test specialist trained April 2026 goes stale when Python 3.14 lands. Need: retraining triggers, version compatibility with customer stacks, canary deployment of new specialist versions, rollback discipline, specialist-version skew handling between panel members. None of this exists; no forcing function yet.
**Related REQs:** REQ-058 (fingerprint-based baseline invalidation — the seed of this pattern)
**Last surfaced:** 2026-04-23

### R-003 — Panel composition drift across deployments
**Category:** Architectural / Cross-customer comparability
**Severity:** MEDIUM
**Disposition:** AWAITING TRIGGER (multiple customers with different panel compositions)
**Description:** Customer A has 5 specialists; Customer B has 7; Customer C has 3. Same Pantheon gate run on two different panels is not the same measurement. Per-deployment baselines with explicit "panel fingerprint" as a distinct axis; verdict interpretation across deployments needs a normalization discipline.
**Related REQs:** REQ-072 (per-hypothesis baseline), REQ-095 (panel parametric schema)
**Last surfaced:** 2026-04-23

### R-004 — Specialist-pool as a sub-platform
**Category:** Architectural / Scale
**Severity:** MEDIUM (grows to HIGH at Pantheon Federal scale)
**Disposition:** AWAITING TRIGGER (Vulcan-1 + first customer fine-tunes)
**Description:** A customer's Pantheon Federal deployment could host hundreds of fine-tuned specialists across languages × domains × compliance tiers. Storage, versioning, deployment orchestration, A/B testing, rollback — specialist registry as its own sub-spec. Gate 4+ concern.
**Related REQs:** REQ-095 (panel parametric), REQ-097 (Pantheon Federal)
**Last surfaced:** 2026-04-23

### R-005 — Continual-learning feedback loop integrity
**Category:** Architectural / ML-safety
**Severity:** HIGH (the long-term vision rests on this working correctly)
**Disposition:** WATCHING (theoretical)
**Description:** User's Q6 "feedback loop from production back to generation" vision requires continual learning, which has known regression modes — catastrophic forgetting, distribution drift, feedback-loop amplification (generator learns from its own outputs). Discipline needed before REQ-093 training-data extraction pipeline is wired to retraining.
**Related REQs:** REQ-093 (drift detection + training-data forward-anchor)
**Last surfaced:** 2026-04-23

### R-006 — Adversarial resilience at the specialist layer
**Category:** Architectural / Security
**Severity:** HIGH for Federal, MEDIUM for Commercial
**Disposition:** WATCHING
**Description:** A customer-specific specialist trained on proprietary or classified code is a high-value target. Attack surface includes: prompt injection, model extraction, training-data recovery attacks, adversarial inputs designed to fool specific fine-tunes. Each specialist deployment needs its own security posture — no pattern yet.
**Related REQs:** REQ-083 (credential resolution), REQ-097 (Pantheon Federal)
**Last surfaced:** 2026-04-23

### R-007 — Model-weights audit trail for Federal
**Category:** Architectural / Federal compliance
**Severity:** HIGH (Federal-mandatory)
**Disposition:** AWAITING TRIGGER (first Federal engagement)
**Description:** Federal mandates proof-of-origin for every byte of model weights. Training data provenance, checkpoint digests, environment fingerprint — all as durable artifacts. Maps onto Pantheon's existing audit-native discipline but specifically for model assets. Partially overlaps R-001 (specialist provenance); may consolidate.
**Related REQs:** REQ-057 (environment fingerprint), REQ-097 (Pantheon Federal)
**Last surfaced:** 2026-04-23

---

## Commercial / Legal / Strategic landmines (not REQ-shaped)

### R-008 — FedRAMP High certification cost and timeline
**Category:** Commercial / Compliance
**Severity:** HIGH (gates Pantheon Federal market entry)
**Disposition:** AWAITING TRIGGER (first serious Federal lead ready to move)
**Description:** FedRAMP High authorization for an AI platform costs ~$500K–$2M and takes 12–24 months of 3PAO-led process. Pantheon Federal is architecturally ready before it's procurable. Paths: partner with an already-authorized host (AWS GovCloud, Azure Government), pursue direct authorization, or operate as a subcontractor under an already-authorized prime. Each has tradeoffs. COL Poindexter / JIOP engagement may surface the right path.
**Last surfaced:** 2026-04-23 (Round 4 landmine inventory, post-Poindexter email)

### R-009 — ITAR / export controls on specialist weights
**Category:** Legal / Compliance
**Severity:** MEDIUM (becomes HIGH with multi-national engagements)
**Disposition:** WATCHING
**Description:** AI models trained on certain corpora (defense-adjacent code, cryptographic implementations, controlled technology documentation) may fall under ITAR or EAR controls. Cross-border specialist sharing between Pantheon deployments becomes a legal minefield. Specialist-catalog partitioning per export classification needed before any multi-national engagement. Domestic-only initial scope sidesteps this, but only for a while.
**Last surfaced:** 2026-04-23

### R-010 — Commercial frontier model price collapse
**Category:** Strategic / Market
**Severity:** MEDIUM
**Disposition:** WATCHING
**Description:** If GPT-5/Claude-5/Gemini-3 pricing collapses to $0.01/call in 2027, the cost argument for locally-trained specialists weakens in the Pantheon Commercial (Old Iron) market. Pantheon Federal is unaffected (sovereignty requirement is orthogonal to price); but Old Iron commercial thesis needs a secondary justification beyond cost — likely audit, compliance, customer-specific fine-tune quality, or operational independence from vendor availability.
**Last surfaced:** 2026-04-23

### R-011 — Government compute procurement lead time
**Category:** Operational / Commercial
**Severity:** MEDIUM
**Disposition:** AWAITING TRIGGER (Pantheon Federal first deployment)
**Description:** Vulcan-1 is 2×3090 (consumer, ~6-week procurement). Federal-scale is H100/B200 clusters with 6–18 month procurement cycles bound to government FY budget windows. Pantheon Federal deployment path is gated by government IT procurement — slow, political, often requires incumbent relationships. Not something we fix; something we plan around. May push Pantheon Federal first-deployment to 2027-2028 even if architecture is ready in 2026.
**Last surfaced:** 2026-04-23

### R-012 — Legal ownership of panel verdicts
**Category:** Legal / Contractual
**Severity:** MEDIUM (becomes HIGH at first compliance audit)
**Disposition:** WATCHING
**Description:** If three specialists disagree and the verdict engine calls a winner via K-of-N, who owns that verdict for compliance purposes? Customer (because they ran it)? Pantheon operator (because the substrate produced it)? Original model vendors (because the specialists are derivatives)? Contract-level question, unsettled. Will be pushed by first serious Federal engagement or first audit against a Commercial Old Iron deliverable.
**Last surfaced:** 2026-04-23

### R-013 — Labor-cost inversion from specialist proliferation
**Category:** Operational / Strategic
**Severity:** MEDIUM (grows with Pantheon Federal adoption)
**Disposition:** WATCHING
**Description:** As customer-specific specialists proliferate, the cost of TRAINING and MAINTAINING the specialist catalog may exceed the cost of running it. Currently cheap because Gauntlet uses commercial models (no training overhead). Federal model changes this equation — fine-tuning runs, evaluation harness maintenance, human ML-engineer time for specialist curation. Unit economics for Pantheon Federal engagements need to absorb this or amortize it across multiple customers.
**Last surfaced:** 2026-04-23

### R-014 — Dual-subscription account-rotation opportunity (Gate 2+ scope) — RETIRED
**Category:** Capacity / Architectural
**Severity:** N/A
**Disposition:** RETIRED 2026-04-23
**Reason for retirement:** Experimenter directive 2026-04-23: "we have enough capacity to do whatever we want." Account-rotation is a solution to a capacity-constraint that doesn't exist at this subscription envelope. Anchoring it created design gravity toward building a feature addressing a non-problem. If capacity becomes a constraint in future (e.g., Pantheon Federal engagements with stricter rate-limit envelopes, Vulcan-1 burst workloads), re-surface under a new registry ID at that time — don't resurrect a retired entry.
**Original framing (preserved for audit):** Experimenter's subscription envelope `2x Claude Max 20x + Gemini Ultra + Codex Pro` was framed as "second Claude Max account sitting idle at Gate 1 scope, rotation opportunity for Gate 2+." Framing was incorrect — capacity headroom at steady state is ~7-10× cluster budget per account; doubling that via rotation addresses no observed or anticipated constraint.
**Last surfaced:** 2026-04-23

---

## How to use this registry

- **Don't let it grow stale.** Quarterly review minimum. Re-check severity and disposition.
- **Promote to REQ when ready.** Architectural items (R-001 through R-007) may graduate to REQ anchors once we have enough implementation or market experience to specify them without guessing. Update disposition to ACTIVE and cross-reference the REQ.
- **Retire when obsolete.** Mark entries RETIRED with a reason (no longer applicable, absorbed by another entry, resolved). Don't delete — retirement itself is audit trail.
- **Add new entries as they're conceived.** User's Round 4 discipline: "call them out as we conceive of them." This registry is the call-out destination.

## Cross-reference to spec

REQs that anchor to the Pantheon Federal deployment profile: REQ-097 (Pantheon Federal deployment profile anchor).
REQs that implicitly contain mitigations for registry items: REQ-052, REQ-057, REQ-058, REQ-072, REQ-083, REQ-092, REQ-093, REQ-095.
Research artifact informing long-term vision: `/Users/mikeboscia/projects/triumvirate/daemon/docs/research/2026-04-23-code-evaluation-and-closed-loop-sdlc.md`.
