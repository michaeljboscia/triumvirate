# Raw peer output: 20-EVIDENCE-BUNDLE-SPEC.md unit 1 (lines 1-47)

Header, Design goals, Bundle directory structure.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

1. **Line 13 states immutability as if it exists,** but lines 1-47 only specify intent. Nothing here establishes GCS enforcement. Given the Phase 2 context, UBLA plus public-access-prevention do not make objects immutable. This needs a retention policy / bucket lock, object versioning with governance rules, or a weaker claim.

2. **Confirmed contradiction.** Line 13 says "Once written, never modified." The lifecycle section says `manifest.json` is created `running` at T+0 and later updated with verdicts and `ended_at`. **The lifecycle should give.** For client-facing proof, write mutable working state outside the evidence bundle, then publish a finalized immutable bundle once.

3. **Directory structure lines 27-43** requires `logs/`, `artifacts/`, `raw/`, and `metrics/nvidia-smi.csv`. The old harness gaps are fixed. But line 43 still only describes `raw/` as "gate-specific"; this range does not require any concrete file under it. **For an isolation/sovereignty claim, `raw/` is load-bearing if tcpdump/strace is the primary proof of network and process behavior. If nothing explicitly populates it, the claim rests on summarized metrics rather than inspectable evidence.**

4. **Line 19's `< 100MB` target is likely incompatible with raw packet capture** for a full gate run. A tcpdump can easily exceed that depending on duration, traffic volume, snap length, and whether payloads are captured. The spec needs capped captures, metadata-only flow logs, rotation and compression rules, or a different storage target.

5. **Missing critical property: verifiability / tamper evidence.** Lines 13-19 cover usability and queryability, but not cryptographic integrity: hashes, signed manifests, provenance, chain of custody, retention enforcement, or reproducible validation. For a client-facing security proof, that absence is material.

---

## GEMINI (strategic angle)

**1. The seven design goals (lines 13-19).** This list is optimized exclusively for internal data ingestion (Supabase, Pythia, lines 16-17) and developer ergonomics (line 18). **It is a data engineering brief, not a security brief.** It is entirely unoptimized for establishing trust with a hostile reader. A skeptical client does not care if the data is "cheap to store" (line 19) or "tool-agnostic" (line 15); they care if it is true.

**2. The single missing property: cryptographic verifiability (non-repudiation).** Claiming a file is "Immutable" (line 13) is meaningless without structural proof. In practice this requires cryptographically signing the `manifest.json` (Sigstore or a dedicated KMS key) and providing an independent, verifiable timestamp proving the artifact existed at the time of the claim and has not been altered since.

**3. The independence problem.** Nothing in lines 1-47 addresses the conflict of interest of a system generating its own security evidence. **To a hostile reader this is a system grading its own homework.** Addressing it requires an out-of-band observer (an independent control plane or infrastructure-level logging) to assemble the bundle, and writing it to a WORM bucket with cloud-provider retention locks rather than a standard GCS bucket (line 5).

**4. Storage destination (line 7).** GCS is the wrong destination for local box execution. Hardcoding `gs://` (line 5) tightly couples the execution environment to cloud infrastructure. **The storage root should be transport-agnostic, defaulting to a local filesystem URI** (`file:///var/run/pantheon/evidence/...`) to decouple completion from cloud synchronization.

**5. Directory structure (lines 25-44). Two bundles are required.**
- **Client security team:** `manifest.json` (27), `summary.md` (28), potentially `artifacts/evaluations/` (41).
- **Internal debugging:** `obsidian-note.md` (30), hardware metrics (33), verbose container logs (34-38), `raw/` captures (43).

Handing a client an `obsidian-note.md` is unprofessional. Handing them raw `strace` and `tcpdump` output (43) is an active security risk that leaks infrastructure details and buries the actual evidence in noise. **A public-facing audit bundle must be minimal, sanitized, and verifiable; the debug bundle stays internal.**

---

## DEEPSEEK (adversarial logic angle)

Asked: a spec that DECLARES a property versus a system that ENFORCES it.

> Declaring immutability is a promise; enforcing it is a mechanism. Practically, a reader who relies on a
> declared-immutable artifact still has to trust whoever controls the storage and the writing system. If the artifact
> is mutable underneath, it can be altered silently, and nothing in the artifact itself reveals the change. The spec
> makes this worse by saying the manifest is created at T+0 with `status="running"` and then updated at the end. That
> is a mutation, so the system contradicts the stated goal.
>
> Minimum structural change a skeptic can check: **make every object write-once and content-addressed, and anchor a
> signed hash of the bundle (or each bundle object) in an external append-only log unavailable to the writer**, for
> example a transparency log or separate ledger. Then the manifest is never rewritten; instead the initial run-state
> record and the final verdict record are separate objects, and the external hash chain proves both were created when
> claimed and have not been altered. Add versioning and/or a WORM lock on the bucket as a second layer, but **the
> external signed hash is the checkable part.**

**Three-peer convergence on the same missing property, reached three ways.** Codex: "verifiability / tamper evidence."
Gemini: "cryptographic verifiability (non-repudiation)." DeepSeek: an external append-only anchor outside the writer's
control. All three also independently identified the T+0-then-update lifecycle as a direct contradiction of design
goal 1.

**The actionable synthesis:** split the mutable run-state record from the immutable published bundle, make bundle
objects write-once and content-addressed, sign the manifest with a key the tested system never holds, and anchor the
signature externally. Bucket-level WORM is the second layer, not the first.
