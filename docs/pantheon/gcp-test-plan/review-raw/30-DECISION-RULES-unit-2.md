# Raw peer output: 30-DECISION-RULES.md unit 2 (lines 128-246)

Decisions 4, 5, 6, 7, 8.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**Verdicts:**
- **Decision 4: KEEP-WITH-EDITS.** Machinery works locally, but "spending GPU dollars" / "GPU gate" language is cloud-era phrasing; replace with "GPU-backed validation" and require local bundle artifacts (136-144).
- **Decision 5: KEEP-WITH-EDITS.** Substrate-agnostic thesis rule survives, but should hang off the current software-validation run, not "Gate 4" (148-169).
- **Decision 6: CUT.** A CapEx trigger for a Pantheon Rack purchase, not a software validation rule (176-196).
- **Decision 7: KEEP-WITH-EDITS.** Closest product claim, but current evidence is insufficiently hard for "isolated." Needs packet-capture evidence, deny-by-default egress proof, artifact hashes (200-217).
- **Decision 8: KEEP-WITH-EDITS.** Survives, but should hang off release-candidate soak/fault validation, not demoted "Gate 7" (224-240).

**Thresholds audited:**
- `5-task dispatch` (142): not throughput, no conflict.
- `4/4 worktrees`, `>= 3/4 merge cleanly` (157): valid software threshold.
- `>= 80%` code validity for validated, `< 50%` falsified (158, 166): leaves 50-79% to inconclusive. OK.
- `<= 15 min median`, `<= 30 min max` (159): wall-clock, no conflict.
- `2-3` hypotheses pass (162): OK.
- `$150-500K`, `$80-500K`, `$500K/year` (178, 184, 192, 196): CapEx/business, cut with Decision 6.
- `<= 5 incidental packets` (209-212): **weak unless packet capture proves destination, protocol, and expected PGA-only path.**
- Decision 8 (228-240): no tok/s conflict, **but no explicit production throughput floor. Decision 8 should add `>= 15 tok/s/stream under 4-way batched load`.**

**Decision 4 locally.** Works. NATS, container, mock-vLLM, Triumvirate startup, end-to-end dispatch are substrate-neutral. Cloud assumptions are only the GPU-cost framing and "GPU gate" wording (138, 144), not the rule itself.

**Decision 7 in detail.** Current evidence requires zero outbound traffic with an allowance, air-gapped swarm completion, evidence bundle via PGA, fresh timestamp (208-212). **Not sufficient to tell a client "isolated" unless H-6.1 is explicitly packet-capture-backed. As written it can read like a firewall/config claim.** It must distinguish:
- **Config claim:** firewall/route/DNS/proxy settings say egress is blocked.
- **Evidence claim:** packet capture and flow logs show no disallowed egress during the run.

Keep the rule, but require pcap/flow artifacts, capture interface names, time window, hash of the evidence bundle, and explicit accounting for the `<= 5 incidental packets`.

**Decisions 5 and 8 survive demotion**, but should hang off named validation artifacts rather than historical gate numbers: Decision 5 off a "core thesis validation run"; Decision 8 off a "release-candidate production readiness bundle" covering soak, concurrency, fault injection, and the 15 tok/s/stream floor.

**Decision 6, owner rule applied.** What it was for: deciding whether to buy enterprise GPU hardware (176-187). Does the problem persist: capacity planning does; self-funded rack purchasing does not belong in this corpus now. Replacement: a customer-funded capacity/procurement policy tied to signed workload requirements, local validation results, and hosted/burst alternatives. Retain the validation prerequisites, not the purchase trigger.

**Missing/fragile dependencies:** gate bundle paths that may be stale (152, 180, 204, 228); "full 4-task canonical swarm" needs a canonical fixture definition (210); "Gate 0 bundle metrics" needs concrete local artifact names (132); "evidence bundle lands via PGA" needs an evidence schema/hash/checklist (211).

---

## GEMINI (strategic angle)

**1. Decision 7 (200-223) is dangerously weak.** It checks that the network is dead (209) and the program did not crash (210), **but it does not verify the utility of the output in a disconnected state. What if agent tools silently fail or fall back to useless defaults when the internet is unreachable?** Add: **a strict artifact parity check against a connected run. The code generated must be functionally equivalent, not just "completed."**

**2. Decision 4 (128-147) actively enables lingering.** By stating "No further Gate 0 runs required unless..." (136), it gives permission to declare the cheap test done and then indefinitely delay the expensive run. **It guards against wasting money on broken code but provides no forcing function to advance.**

**3. Decisions 5 and 8 under the rebuild.**
- Decision 5 (148-174): instead of a binary thesis check, becomes a **unit economics baseline.** Line 159's time limits shift from pass/fail to a cost-per-swarm calculation informing pricing.
- Decision 8 (224-246): instead of a production gate, becomes an **OpEx stress test.** Line 234's no-memory-leaks is not just stability, it is whether long-running tasks force over-provisioning of expensive memory on rented nodes.

**4. Decision 6 (176-199)** was the CapEx trigger to buy a physical $80K-$500K rack. Dead under rent-first. **Replace with an Instance Reservation / Committed Use Discount trigger:** the decision is no longer "do we buy metal" but "at what sustained utilization do we lock into a 1-year contract instead of paying on-demand."

**5. THE GAP. There is absolutely no rule guarding against budget bleed.** Decision 6 used to be the hard stop because it required writing a massive check. **Under rent-first you could burn $150K across a year of "promising" iterative GCP runs and nothing in this document would stop you.** Needs a cumulative OpEx burn limit that forces a hard pivot or shutdown if crossed without revenue.

---

## DEEPSEEK (adversarial logic angle)

Asked: an isolation gate checks that tasks COMPLETED while disconnected, not that output was correct. Name the failure mode.

> **Failure mode: silent functional degradation.** Specifically, the agent completes with a nominal success status while
> tool failures are masked by cached defaults, skipped retrieval, or empty-but-valid-shaped results.
>
> **Minimum additional check:** validate task outputs against expected/golden results or semantic invariants, not
> merely that the run completed. Compare outputs byte-for-byte or semantically against known-correct baselines, or
> assert that required retrieval-dependent fields are non-empty and traceable to live sources.
>
> **Yes, that check has its own catch:** the baseline/golden outputs can themselves be stale or contaminated if they
> were produced with the same degraded fallbacks, and semantic thresholds can be tuned loosely enough to still admit
> worthless results. **The check closes the completion-only gap, but the oracle's quality must itself be independently
> verified.**

**Two-peer convergence, arrived at independently.** Gemini from reading the rule ("what if agent tools silently fail
or fall back to useless defaults"), DeepSeek from the logic of the pass condition. **Both propose the same fix, a
parity check against a known-good run, and DeepSeek adds the catch Gemini did not: the baseline itself can be
contaminated by the same degradation, so the oracle needs independent verification.**

That caveat matters for implementation: **the connected baseline must be captured and hashed BEFORE the isolation
test, and its own correctness established separately**, or the parity check just compares two degraded runs.
