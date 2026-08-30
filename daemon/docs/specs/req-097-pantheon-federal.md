# REQ-097 — Pantheon Federal

**Status:** Architectural-commitment anchor, not implementation-blocker for Gate 0/1/2.
**Timing:** Gate 3 (Vulcan-1 sovereign per REQ-088) is the earliest gate validating Federal-tier operation.
**Cross-references:** REQ-052, REQ-057, REQ-083, REQ-088, REQ-092, REQ-093, REQ-095.

## Architectural commitment

Pantheon is deployable in three progressively-sovereign profiles, with Federal as the natural end-state of the architecture's on-prem, cross-provider, audit-native, specialist-composable design choices.

### (a) Pantheon Core

The orchestration substrate itself: panel parametric over N≥3 per REQ-095, cross-lineage, execution-grounded, evidence-bundle-auditable. Runs against commercial CLIs (Gate 0.1/0.2), local MLX (Gate 0.3), or LAN vLLM (Gate 0.4) interchangeably.

### (b) Pantheon Commercial

Pantheon Core deployed for regulated commercial customers (banking, manufacturing, healthcare) per the Old Iron Software delivery pattern. Specialists optionally fine-tuned on customer corpora; data governance per customer contract; no FedRAMP requirement.

### (c) Pantheon Federal

Pantheon Core deployed sovereignly for defense/IC/civilian-federal customers.

- All commercial API calls disqualified by default
- Panel populated entirely by locally-trained specialists running on customer-controlled hardware
- FedRAMP High / DoD IL4+ compliance posture baseline
- Air-gap-compatible operation
- ITAR / export-control-aware specialist-catalog partitioning

## Specialist-composability (extends REQ-095 panel-parametric schema)

Panel members identified by **role + model-provenance + fine-tune identifier + deployment tier**, not by fixed "claude/codex/gemini" identities.

Federal deployments may populate the panel with up to N=7+ customer-specific specialists fine-tuned on the engagement's:

- Codebase
- Language/framework subset
- Clearance-bounded corpora
- Module-type specialization (auth-module specialist, persistence-layer specialist, cryptography-module specialist, API-boundary specialist, etc.)

Substrate handles arbitrary panel composition. Panel selection is an operator/customer-engagement-level decision, not a Pantheon-level decision.

## Federal-specific architectural commitments

1. **REQ-083 strict-mode resolution** MUST be active in Federal deployments, with required tier three or higher (OS keyring minimum); no fallback to process-environment variables.

2. **Evidence bundle redaction per REQ-052** extended with cleared-content-classification-level tagging (U, CUI, S, TS — placeholder vocabulary; actual classification vocabulary negotiated per engagement).

3. **Daemon-side OTel integration per REQ-092** defaults to emit-to-local-store, never to cloud telemetry backends, without explicit operator opt-in.

4. **Drift-detection pipeline per REQ-093** supports air-gapped operation — telemetry aggregation and trend computation run locally, no external service dependencies.

5. **Fine-tune specialist provenance as audit artifact** — each specialist's training corpus manifest, weight digest fingerprint, training environment fingerprint, and retraining history preserved in the evidence-bundle-adjacent `specialists/` tree.

6. **Panel composition recorded in manifest.json (REQ-034)** with role, specialist identifier, fine-tune identifier, and weight digest fingerprint fields per participant — verdict interpretation requires knowing WHICH specialists issued the verdict, not just the aggregate.

## Commercial positioning consequence

Fort Liberty, CDAO, and JIOP engagements land directly on "Pantheon Federal" rather than "Pantheon Core with federal-friendly architecture" — the product tier has a name, a committed posture, and a roadmap even if implementation timing is Gate 3+/Vulcan-1-era.

Old Iron Software commercial engagements default to Pantheon Commercial; upsell path to Pantheon Federal exists for customers who later need sovereign-tier deployment.

## Rationale

User architectural commitment in Round 4 Gate 1 scope discussion following COL Poindexter XVIII ABN CORPS introduction:

> "we don't know where any of this is going to go — and we have SOME time to develop the intellectual pathways to think through the problems and challenges and landmines and roadmap items that we KNOW will be the path we're on — might as well call them out as we conceive of them."

REQ-097 is that call-out for the Pantheon Federal deployment tier.

Pantheon Federal is not a cousin of the commercial product — it is the sovereign end-state of the same architecture the commercial product approaches from the other direction.
