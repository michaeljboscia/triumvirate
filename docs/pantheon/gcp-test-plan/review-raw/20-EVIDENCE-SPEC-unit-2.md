# Raw peer output: 20-EVIDENCE-BUNDLE-SPEC.md unit 2 (lines 48-321)

Required file schemas: manifest.json, summary.md, cost-report.json, obsidian-note.md, metrics.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. `manifest.json`: populated vs aspirational**
- **Actually populatable by harness:** `schema_version`, `run_id`, `gate`, `started_at`, likely `ended_at`, `duration_sec`, GCP location/machine/accelerator/provisioning fields, `models_used`, `hypotheses_tested`, raw `verdicts`, bundle `links` if paths are deterministic (lines 56-83, 91-98).
- **Aspirational or unsafe:** `experimenter` as identity evidence (60), `triumvirate_version`/`git_commit` unless captured from the running artifact (61-62), `prior_runs_referenced` unless lineage is enforced (82), `decision_rules_applied`/`decision_rule_outcomes` unless rule execution is machine-evaluable (83-90), `total_cost_usd` at finalization (91), `evidence_bundle_size_mb` unless computed post-write (92).
- **`confidence: 0.85` is invented** unless there is a documented scoring model. Nothing in lines 84-90 explains computation, inputs, calibration, or provenance. For client evidence, remove it or replace with traceable rule status.
- **The hardcoded Gate 2 / "buy 2x 3090 NVLink" example is a problem in a schema spec.** It teaches a cancelled purchase decision as schema truth (57-59, 84-87).

**2. Required fields (line 102)**
- **Bad required field: `total_cost_usd`.** If billing export is delayed, it cannot be required at run finalization.
- **Missing for client-facing evidence:** immutable artifact hash/digest, generator identity/version, source git dirty/clean state, schema validation status, evidence completeness status, cost status/provenance, metric file list with checksums, rule version/threshold version, signed or append-only manifest finalization marker.
- `ended_at` is fine, but the spec must distinguish `manifest.started.json` from a finalized manifest, or use append-only finalization. Line 52 says it is updated mid-lifecycle.

**3. METRICS THRESHOLD BUG. Yes, the example encodes a PASS that fails the standing production floor.**
- Median is **12.4** tok/s/stream, below the required **15** under 4-way batched load (line 287).
- **Worse, the example is single concurrency (`concurrency: 1`, line 275), so it does not test the standing policy condition at all.**
- The spec teaches **10** tok/s/stream as the pass threshold (lines 297-302). That conflicts with the production floor.

**4. `cost-report.json`**
- Line 188 assumes `label-based query on billing export`.
- If billing export is not already enabled, the report cannot exist from that method.
- Even if enabled, **Google says Cloud Billing export is automatic "throughout the day," not real-time, and initial backfill can take up to five days** before recent usage appears.
- **So at run finalization, authoritative cost is necessarily unknown.** The client artifact should say `cost_status: pending_billing_export`, not invent `total_cost_usd: 0.86` (line 185).

**5. `obsidian-note.md`:** not as a required client-facing bundle file. It is vault/internal workflow material by design (192-194). `rtx-3090-proxy` ties evidence to the cancelled purchase rationale (223). `significance: 3` is subjective and unexplained (226). `## Mike's notes` is explicitly personal annotation space (255-256). Keep it out of the client bundle, or generate it separately as private operator notes.

**6. `nvidia-smi.csv`:** a 30-second interval is adequate for coarse utilization, rough memory residency, thermal drift, and broad power shape (311-318). **It misses** short GPU stalls, bursty saturation, memory allocation spikes, power throttling transients, PCIe/NVLink transfer bursts, per-process attribution, kernel-level latency, and sub-request contention. A reviewer would want per-process GPU usage, command provenance, driver/CUDA versions, finer sampling or DCGM metrics, and correlation IDs tying telemetry to specific test windows.

---

## GEMINI (strategic angle)

**1. Schema survival.** The schema is obsolete. `decision_rules_applied` and `decision_rule_outcomes` (83-90) and hardware-specific tags like `rtx-3090-proxy` (223) must be purged entirely. The "hypothesis" structure must be replaced with "Security Controls Validated" and "Pilot Economics." The schema must shift from internal hardware justification to external isolation proof.

**2. `confidence: 0.85`** (line 87) is completely fabricated. In a system without a probabilistic decision model it is pseudo-scientific padding. **Submitting baseless confidence scores to a security team will immediately destroy the credibility of every actual metric in the bundle.**

**3. New hypotheses:**
- H-1: The inference container drops all outbound internet traffic.
- H-2: Model weights are loaded exclusively from internal, authenticated VPC buckets.
- H-3: Qwen2.5-72B maintains >10 tok/s at a unit cost under $1.00 per 10k tokens on GCP infrastructure.

**4. `summary.md` changes.** Cut "Decision rules applied" (137-138) and the academic "Hypotheses tested" format (121-130). A client security team does not care about internal decision models. Add "Network Isolation Assertions" and "Pilot Pricing Projections."

**5. `obsidian-note.md`** (196-257) absolutely does not belong in a client artifact. `significance: 3` (226) and `## Mike's notes` (255) explicitly expose that this bundle was originally a personal scratchpad built to justify a hardware purchase.

---

## DEEPSEEK (adversarial logic angle)

Asked: name the two distinct defects in the metrics example, and which is more dangerous in a template.

> The two defects:
>
> 1. **Wrong pass/fail threshold.** The example uses `tokens_per_second_per_stream_min: 10`, but the production floor
>    is 15. So `12.4` gets a `PASS` even though it is below the real requirement.
> 2. **Wrong measurement condition.** It is measured at `concurrency: 1`, while the policy requires 4-way batched
>    concurrent load. The result is not measuring the condition the policy is about.
>
> **More dangerous in a copied template: the concurrency mismatch.** A wrong threshold is a visible numeric error; a
> wrong load condition invalidates the entire measurement while still producing a plausible-looking PASS. People
> copying the template will repeat the same structurally meaningless test setup without noticing.

**Two-peer convergence with Codex,** which found both defects independently by reading the file. DeepSeek adds the
ranking, and the ranking is the useful part: **a visibly wrong number gets caught eventually, a silently wrong
experimental condition propagates through every run that copies the template and still looks like a pass.**

---

## REVIEWER DISSENT (recorded per the standing rule: do not delete without asking what it was for)

**I disagree with Gemini's point 4 on cutting the hypothesis structure, and the disagreement matters.**

The hypothesis format exists to **pre-register a prediction before the test runs**, so a result cannot be
rationalized after the fact. That is the single most epistemically sound thing in the original corpus, and it is
precisely the discipline whose absence produced everything else in this review. Cutting it because the current
hypotheses are about a cancelled purchase confuses the **content** with the **structure**.

**Replace the content (H-2.1 "70B-Q4 local inference is usable"), keep the structure (a pre-committed prediction with
a threshold, tested, then marked PASS or FAIL against that threshold).** Gemini's own suggested replacements in its
point 3 are themselves hypotheses, which rather makes the case.

Same treatment for the other two deletion candidates:

- **`decision_rule_outcomes`** existed to create an audit trail from a verdict back to the evidence that produced it.
  That is worth keeping. Cut only the fabricated `confidence` float, and replace the structure with: rule id, rule
  version, threshold value, measured value, and the resulting pass/fail. Traceable rather than invented.
- **`obsidian-note.md`** existed for knowledge capture, so runs compound into a searchable vault instead of evaporating.
  That is a real purpose. Move it out of the client bundle into an internal sidecar; do not delete the capability.
