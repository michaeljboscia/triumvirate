# Raw peer output: gate-6-airgap-sanity.md unit 1 (lines 1-107)

Why this gate matters, What it validates / does NOT validate, Hypotheses, Pre-run checklist, Step 1.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

**This document is the product claim.** It is what a prospect's security team reads.

---

## CODEX (engineering angle)

**1. H-6.1 is falsifiable but internally diluted.** Thresholds at line 45: *"Zero outbound attempts detected -> PASS," "1-10 attempts -> investigate," "> 10 attempts -> fundamental architectural leak."* **That does not match line 3's "Any packet leaving the VM = fail" or line 22's "ZERO outbound HTTP/HTTPS/DNS."** It also measures attempts *"that reach the firewall"* (43), not all packets.

**2. H-6.2 is only partly falsifiable.** Threshold: "4/4 tasks complete successfully -> PASS", any stall or timeout fails (54-56). **It checks completion and eval pass, but not baseline equivalence: no pre-captured connected baseline hash, no output hash, no independent correctness proof** (52).

**3. H-6.3 is falsifiable but supports restricted egress, not disconnected operation.** "Bundle lands -> PASS" (62-64). Because it explicitly allows GCS via PGA (60), **it validates controlled Google-private egress, not "ZERO outbound traffic."**

**4. Boundary honesty is weak.** "What it validates" claims ZERO outbound HTTP/HTTPS/DNS (22), local weights (23), local Docker cache (24), no telemetry (25-27), local retrieval (28). "What it does NOT validate" excludes hardware representativeness, drift, and cloud-required third-party integrations (31-35). **It quietly omits exclusions the claim depends on: IPv6 coverage, metadata server access, DNS/NTP paths, packet-capture scope, and correctness/equivalence of disconnected outputs.**

**5. Zero-versus-five.** In this range the zero side appears at **line 3** ("ZERO outbound traffic", "Any packet leaving the VM = fail"), **line 14** ("does not attempt any outbound network connection"), **line 22** ("ZERO outbound HTTP/HTTPS/DNS"), **line 46** ("Zero outbound attempts detected -> PASS"). The five threshold is not in this range. **The load-bearing number here is zero.** The document should instead say it validates *"no observed unauthorized public-internet egress under stated capture coverage,"* while separately allowing stated PGA paths and requiring packet-capture evidence plus baseline output equivalence.

**6. Step 1 does not apply firewall rules at all in this range.** It provisions with `--no-address` (92-105), **but no deny/allow egress rules are created or attached.** Unaddressed paths: **IPv6** (no `::/0` denial), **metadata server** `169.254.169.254` (not blocked or monitored), **DNS** (no resolver path pinned or blocked), **NTP** (not blocked or captured). Existing VPC default egress may still allow traffic unless rules exist outside this range.

**7. Machine/flag issues.** `g4-standard-32` at line 93 is **not a valid machine type**. The accelerator pairing at 94 depends on a valid machine/accelerator SKU pairing. `--max-run-duration=60m` (96) is CLI-shape sensitive and this excerpt gives no validation command.

**8. Pre-run checklist (70-74)** items exist **as checklist assertions only**; the range shows no proof for any of them.

**9. Capture methodology.** tcpdump is mentioned only in the prediction: *"After firewall lockdown applied... tcpdump on VM"* (43). Step 1 says the firewall is applied *"DURING the test, after stack is up"* (91). **Interface is not specified.** It would miss provisioning and startup traffic before lockdown, traffic outside the captured interface, metadata/DNS/NTP paths if filtered or on unexpected interfaces, and anything allowed by PGA unless filters explicitly include it.

---

## GEMINI (strategic angle)

**1. "Why this gate matters" makes a false promise.** Line 14 claims Pantheon *"does not attempt any outbound network connection,"* yet lines 59-60 and 71 explicitly test outbound GCS uploads via PGA.

**Rewritten core claim:** *"When deployed in a sovereign environment, Pantheon operates completely isolated from the public internet, restricting all egress exclusively to customer-authorized private endpoints."*

**2. Split into two documents.** A runbook must be a linear executable script. Combining cloud and local modes forces constant branching (gcloud provisioning versus local scripts, PGA rule verification versus literal physical isolation). **One runbook with two modes introduces execution drift and dilutes the audit trail for security teams.**

**3. H-6.2 must assert deterministic functional correctness.** Lines 52-55 assert the swarm completes and tasks pass eval without defining the eval's rigor against silent degradation. It should read: *"All tasks pass evals that strictly require local retrieval and inference to succeed, proving the system did not silently fall back, skip steps, or return validly-shaped empty results."*

**4. The `<= 5 packets` allowance.** *What it was for:* a lazy allowance for unavoidable OS-level noise (DHCP, ARP, NTP, metadata lookups) or background telemetry firing before the firewall clamps down. *Does the problem persist?* Yes, OS defaults remain. **But a bare packet count is indefensible to an auditor. Replace it with an explicit protocol/destination whitelist: zero unclassified packets allowed, with explicit exceptions only for named local-subnet OS management traffic.**

**5. The strongest objection, as a client CISO:**

> *"You intentionally punched a hole for Private Google Access to upload evidence bundles. How do you cryptographically or programmatically guarantee that those payloads sent to GCS do not exfiltrate our proprietary context, weights, or PII through that allowed channel?"*

---

## DEEPSEEK (adversarial logic angle)

Asked whether that CISO objection is fatal or answerable.

> **Answerable, but fatal if the vendor just says "trust us."** The key is making the allowed channel a constrained,
> auditable, **write-only pipe.**
>
> **Minimum controls:**
> - **Client-visible bucket/endpoint policy:** the only egress is a private-link endpoint to a **client-controlled**
>   bucket, with a policy allowing only `PutObject` to a fixed evidence prefix. Flow logs confirm no other routes.
> - **Least-privilege uploader:** a separate process identity that can read **only** the evidence directory, not model
>   weights, databases, source, or PII. It enforces an allowlist of expected filenames, types, and sizes, and aborts
>   on anything unexpected.
> - **Manifest and hashes:** every evidence file hashed and listed before upload; the client verifies uploaded object
>   names, sizes, and hashes afterwards. **Smuggled data appears as an unexpected file and breaks the manifest.**
> - **Remote attestation / measured boot:** binds the running test and uploader to a reviewed image, so the client can
>   verify the uploader is actually the restricted thing described.
>
> **Must be client-verifiable rather than vendor-asserted:** the network path and bucket policy (client audits flow
> logs and endpoint policy), the uploaded object list and hashes (client checks bucket against manifest), and the
> attestation report if the vendor controls the host.
>
> **The one thing that cannot simply be asserted is that the uploader has no access to sensitive data.** That must be
> shown through code/image review or, preferably, remote attestation.

**This is the most consequential exchange in the review.** The gate's own evidence channel is an exfiltration channel,
by design, and the document never acknowledges it. **The fix is not to close the channel but to constrain it into a
write-only pipe the client can audit**, and crucially **to point it at a client-controlled bucket rather than ours**,
which inverts who holds the evidence and removes the vendor from the trust path.
