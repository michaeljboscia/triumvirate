# Raw peer output: 20-EVIDENCE-BUNDLE-SPEC.md unit 4 (lines 391-end)

Storage economics, Retention policy, Schema versioning, What this spec enables.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Line 406 arithmetic is internally inconsistent.** The table totals **~672 MB per full Gate 0-7 run**. `100 runs/year` is **~67 GB/year, not ~5 GB/year**. At `$0.020/GB/month`, holding that first year's 67 GB for a year is **~$16/year, not $1.20/year**. The $1.20 figure only matches ~5 GB held all year.

**2. Lines 402 and 406 contradict design goal 7's 100 MB cap.** Gate 7 claims 500 MB. If the isolation gate must include raw packet captures in `raw/`, the claimed 10-20 MB early-gate sizes are not credible without a capture duration, interface rate, compression rule, and cap behavior. **A tcpdump-bearing bundle can easily exceed 100 MB.**

**3. "Effectively free forever" (406) is false as written.** Omits Class A/B operation charges, retrieval charges for non-Standard classes, network egress, lifecycle/rewrite operations, early-deletion charges, and versioned-object storage if bucket versioning is used for immutability.

**4. Nearline after 90 days (413)** is plausible only for rarely-read data, but the spec ignores **Nearline's 30-day minimum storage duration and retrieval fees**. It also needs explicit lifecycle behavior under retention/bucket lock. Retention policies prevent deletion but do not fix the manifest mutation problem.

**5. "Retained forever" / "NEVER delete" (412, 414) is not universally valid.** Conflicts with customer data destruction, legal hold release, privacy deletion, contractual retention limits, and regulated data minimization.

**6. `harness/migrations/` (425) does not exist.** The migration location is aspirational.

**7. "Downstream consumers must handle both" (424) is not actionable** when zero consumers are deployed. There is no compatibility contract to enforce.

**8-14. "What this spec enables" (432-444), item by item:**
- **Semantic search (436): FALSE as written.** The spec defines bundle shape; it does not deploy Pythia ingestion.
- **Structured queries (437): FALSE as written.** Bundle files do not imply Supabase tables, ingestion, schema, indexes, or deployed consumers.
- **Human-readable archive (438): PARTIAL.** Markdown/JSON are readable, but Obsidian integration is not delivered.
- **Reproducibility (439): OVERSTATED.** "See exactly what config produced what result" is **auditability, not reproducibility**. Mutable manifests and unenforced immutability weaken it further.
- **Cost accountability (440): FALSE as written** unless the bundle captures actual billing/SKU data per run.
- **Decision audit trail (441): OVERSTATED.** Depends on correct metrics and deployed decision automation, and the canonical metrics example fails policy.
- **Knowledge moat (442): marketing language, not a property delivered by the spec.** Competitors would not need to re-run every experiment if methodology, outputs, and conclusions are exposed.

**Bottom line:** this final unit overclaims heavily. Storage math is wrong, lifecycle/retention is legally and operationally underspecified, schema migration points to a missing directory, and most "enables" claims depend on undeployed downstream systems.

---

## GEMINI (strategic angle)

**1. "What this spec enables" (441-442).**
- **Item 6 (RTX Pro 6000 audit):** the purchase was cancelled. This is hallucinated future-history serving as a placeholder for a process that does not exist.
- **Item 7 (knowledge moat):** a comforting story and pure hype. **"A pile of local, disorganized JSON test runs is not a moat; it is digital hoarding. Competitors don't need your failed configuration history to beat you; they just need a working implementation."**

**2. "Retained forever" / "NEVER delete" (412-414).**
- **Completely incompatible with a data sovereignty and privacy product.** You cannot promise clients privacy while hoarding their adjacent pilot data in perpetuity.
- **What it was FOR:** overcompensation to justify the moat narrative and to prove compute spend was not wasted ("represent paid compute", line 414).
- **What replaces it:** a strict TTL deletion policy and a provable data destruction protocol.

**3. "Guard them like production data" (444).** Purely decorative security theater. With no cryptographic signing, no immutability enforcement, and Track A writing to standard local disk, the sentence provides zero actionable engineering constraint. **You cannot guard a mutable local JSON file with adjectives.**

**4. Closing voice.** Hype is rampant: "Effectively free forever" (406), "NEVER delete" (414), "your test history IS the moat" (442). Required tone is clinical, precise, and legally unambiguous. A client's security team needs compliance boundaries, retention limits, and zero-trust data handling, not a startup manifesto.

**5. What the rewritten tail should say.**
- **Artifact schism.** *Client artifact:* sanitized, cryptographically signed, delivered, then purged from local systems on successful transfer. *Debug artifact:* internal, scrubbed of client data, local disk, short aggressive TTL before automated deletion.
- **Local storage rotation** replaces "Storage economics": disk quota management and log rotation so the host does not run out of space.
- **Provable destruction** replaces "Retention policy": the exact mechanism guaranteeing client pilot data is destroyed locally.

---

## DEEPSEEK (adversarial logic angle)

Asked: name the conflict between retain-forever and provable destruction, and whether policy can resolve it.

> **Conflict:** the "provable destruction of client data on request" guarantee directly contradicts the policy that
> "bundles are never deleted / retained forever," since those bundles contain client pilot artifacts. **Policy cannot
> resolve this. It IS the contradiction.** Treating history as a moat is just a rationale, not a reconciliation.
>
> **Resolution:** requires structural change. The minimum change is to **separate client data from immutable
> evidence**: make evidence bundles contain only non-client data (metadata, hashes, compute logs, anonymized metrics),
> and store all client pilot artifacts in a separately encrypted, deletion-capable store, so client data can be
> provably erased without touching the evidence archive.

**Three-peer convergence, and it resolves the two-bundle question from unit 1.** Unit 1's split was proposed on
presentation grounds (do not hand a client your `Mike's notes`). DeepSeek shows the split is **structurally required**,
not merely tasteful: the sovereignty product cannot be delivered without it, because two policies the corpus states as
absolutes are mutually exclusive while client data sits inside the evidence archive.

**The split is therefore by data ownership, not by audience:** non-client evidence can be immutable and retained;
client artifacts live in a separately encrypted, deletion-capable store with a TTL and a destruction certificate.
