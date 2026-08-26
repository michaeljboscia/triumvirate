# Isolation Validation

**Status:** rewritten 2026-08-26 against a three-unit, three-peer review of the 2026-04-18 original
**Review record:** `../REVIEW-PROGRESS.md` and `../review-raw/gate-6-unit-*.md`
**Original:** git at `fe6e0f9:docs/pantheon/gcp-test-plan/runbooks/gate-6-airgap-sanity.md`

> **This document is the product claim.** It is what a prospect's security team reads to decide whether to believe
> the sovereignty pitch. It was rewritten rather than edited because its central claim was false on its own terms,
> its passing threshold left room to exfiltrate a private key, and the system was auditing its own isolation from
> inside itself.

---

## 1. Two different claims, two different documents

The original tried to be one runbook for both, and the result made a literal air-gap claim while testing something
materially weaker.

| | **Cloud restricted-egress validation** | **Air-gap validation** |
|---|---|---|
| Where | a cloud VM | the local box, physically disconnectable |
| Strongest honest claim | no public-internet egress; outbound blocked except to named private endpoints | no network path at all |
| Enforcement | inline egress gateway | the cable is out |
| Use it for | most engagements, where the client deploys in their own cloud | clients who require literal isolation |

**Do not call the cloud test an air-gap proof.** The wording that is defensible on a cloud VM:

> "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to named private
> endpoints, each enumerated below. During the measurement window, every packet observed was attributable to one of
> those flows."

The original's opening said the system *"does not attempt any outbound network connection"* while the same document
tested an outbound upload. **A prospect who reads to the end finds that contradiction themselves.**

## 2. The three things a client can check

If this gate is the product, three claims carry it. Everything else in this document is operational detail that
exists for us, not for them, and lives in the appendix.

1. **Zero unauthorized packets**, with every permitted flow enumerated and inspected.
2. **Cryptographic attestation** of the runtime and the evidence bundle.
3. **A write-only pipeline to a client-controlled sink.**

Each is stated below as a claim, a method, and what the client can verify independently.

---

## Claim 1: zero unauthorized egress

### The threshold, and why it is not a number

**Pass condition: zero packets that are not attributable to an enumerated, permitted flow.** Not a count.

> **The original passed at fewer than five outbound packets, "allowing for incidental retry noise."** At a 1500-byte
> MTU a packet carries roughly 1460 bytes, so **four packets is about 6 KB. An SSH or TLS private key is under 2 KB.**
> A vault export, API tokens, or a seed phrase all fit comfortably inside the allowance.
>
> The problem is not that the number was too high. **A packet count is orthogonal to intent.** It models noise as a
> quantity, so anything that fits the budget passes, and "incidental" becomes an unfalsifiable label for whatever
> those packets were. A tolerance is the wrong *shape* of control, not the wrong value.
>
> **What the allowance was for:** genuine OS-level noise, DHCP, ARP, NTP, metadata lookups. That noise is real and
> still happens. **It gets handled by enumerating and permitting those flows explicitly, not by tolerating an
> unexamined quantity of them.**

Every permitted flow is declared in advance with destination, protocol, port, and purpose. Anything else is a failure,
regardless of how little of it there was.

### The observer must be outside the thing observed

**This is the change that matters most, and it is structural.**

The original ran `tcpdump` inside the VM under test, wrote the capture to that VM's own disk, then read it back and
computed the verdict on the same machine. **Observer, observed, evidence store, and adjudicator were one entity.**

That method provides **self-attestation, not proof.** It holds only if the host's OS, kernel, firewall, capture tool,
disk, and verdict code are all trusted and only the *workload* is suspect. It fails completely if the host itself
could be compromised, because whoever controls the host can stop the capture, edit the pcap, or forge the verdict.

**Requirement: capture and adjudication happen outside the tested machine's trust boundary.**

**And the obvious version of that fix is insufficient.** A passive tap or port mirror only sees packets that reached
the wire, so it **cannot show blocked attempts, which are the most persuasive half of the evidence.** The external
component must be an **inline egress gateway that is itself the enforcement point**, so it logs what it refused as
well as demonstrating what never left.

**Cost, stated honestly:** additional infrastructure, a topology change, confirming no alternate egress path exists,
added latency, a new single point of failure, and the external observer itself needs securing and auditing.

### Coverage must be stated, including what was not covered

- **Both address families.** The original denied `0.0.0.0/0` and never `::/0`, and its capture filter was IPv4
  oriented, so **IPv6 was outside both the control and the measurement.**
- **Named interfaces**, not `-i any` with nothing validated.
- **Full payload capture** (`-s 0`); the original relied on a default snap length that may truncate.
- **Every filter exclusion justified in the artifact.** The original silently excluded two address ranges. An
  unexplained exclusion in an evidence document is exactly what an auditor probes first.
- **The metadata server, DNS, and NTP** each explicitly addressed rather than unmentioned.

### What the measurement window does not cover

**Preparation happens before lockdown, and everything fetched then is invisible to this test.** In the original that
was three container images, the task fixtures, and the retrieval corpus.

That is a practical necessity unless everything is pre-baked, **but it must be stated rather than left implicit.**
Bound the trusted pre-provisioning window explicitly, and pin every artifact that enters during it by digest, so what
was loaded is at least identifiable even though it was not observed leaving.

**Build-time behavior is likewise outside this window.** A dormant callback compiled into a base image will not fire
during a short test. Say so.

---

## Claim 2: cryptographic attestation

**NOT BUILT.** Specified here because the claim depends on it and because stating it as a requirement is more honest
than omitting it.

- The evidence bundle's objects are content-addressed, with hashes in a manifest, per `../20-EVIDENCE-BUNDLE-SPEC.md`.
- The manifest is **signed with a key the tested environment never holds.**
- The signature is anchored in an append-only log outside the writer's control.
- **Remote attestation binds the running workload and the uploader to a reviewed image**, so the client can verify
  the uploader is actually the restricted thing this document describes.

**The last item is the one that cannot be replaced by assertion.** That the uploader has no access to sensitive data
must be shown by code or image review, or by attestation. A vendor statement is not evidence.

---

## Claim 3: a write-only pipeline to a client-controlled sink

**This answers the objection a security officer asks first, and the original answered it in no respect.**

The gate proves isolation by denying egress, then deliberately keeps one path open to upload the proof. **That proof
channel is a hole in the thing being proved.** The original uploaded recursively from inside the tested VM, using
that VM's own service account, to a bucket we control, with no manifest check, no size check, and no validation
boundary, so a symlink inside the evidence tree would have been followed.

Requirements:

1. **The sink is a client-controlled bucket**, not ours, with a policy permitting only object creation under a fixed
   evidence prefix. **This inverts who holds the evidence and removes us from the trust path**, which is the single
   highest-leverage change in this document.
2. **A least-privilege uploader identity** that can read only the evidence directory and nothing else: not weights,
   not databases, not source, not client data.
3. **An allowlist of expected filenames, types, and sizes**, aborting on anything unexpected, with symlinks not
   followed.
4. **Hashes computed before upload**, so smuggled content appears as an unexpected object and breaks the manifest.

**Client-verifiable rather than vendor-asserted:** the network path and bucket policy (they audit their own flow logs
and IAM), the uploaded object list against the manifest (they inspect their own bucket), and the attestation report.

---

## 3. Does it still work while isolated?

**Isolation is worthless if the isolated system silently stops working.**

The original passed when the task suite *completed* while disconnected. Agent tooling degrades quietly when the
network is unreachable: it falls back to cached defaults, skips a retrieval step, or returns an empty-but-well-formed
result the surrounding code accepts. **A disconnected run can complete cleanly, produce worthless output, and pass.**

**Pass condition: output is functionally equivalent to a connected baseline.**

**The baseline must be captured and hashed BEFORE the isolation run, with its own correctness independently
established.** A baseline produced with the same degraded fallbacks makes the comparison worthless: you would be
comparing two broken runs and finding them equal.

Evaluations must be constructed so they **cannot pass without local retrieval and inference actually succeeding.**

---

## 4. Verdict

**PASS** requires all three claims plus section 3. Each is independently falsifiable.

| Condition | Pass |
|---|---|
| Unauthorized packets | zero, every permitted flow enumerated and attributed |
| Coverage stated | both address families, named interfaces, exclusions justified |
| Observer independence | capture and adjudication outside the tested machine |
| Attestation | manifest signed with an externally held key, anchored externally |
| Upload channel | write-only, least-privilege, client-controlled sink |
| Functional equivalence | matches a pre-established, independently verified baseline |

**INCONCLUSIVE** is a real outcome and has a branch: if coverage cannot be established, or the baseline's correctness
is unverified, or the external observer was unavailable, **the result is inconclusive and the claim is not made.**
The original had no inconclusive branch at all, which meant every run had to resolve to pass or fail.

**On a FAIL, do not narrow the claim to fit the result.** Amending a threshold after seeing evidence is exactly what
`../30-DECISION-RULES.md` forbids.

## 5. On a PASS

Not "proceed to soak testing." **Passing the product claim should produce the client-facing artifact**: seal the
evidence bundle, generate the attestation, and deliver it to the client-controlled sink.

Two sentences from the original are deleted rather than softened: *"Sovereign claim validated. Ready to ship"* and
*"audit-defensible."* **Neither was supported**, and they are precisely the sentences a prospect would quote back.

---

## Appendix: operational detail

Everything below exists for us, not for the client.

**Teardown.** Delete the firewall rules and the instance. The original's teardown **does** run, unlike gate-0's, but
it is **not transactional**: a failure between the two rule deletions leaves the VM up with one rule removed, and a
failure after both leaves it with neither. Neither intermediate state is safe and nothing detects them. Make teardown
idempotent, verify the end state, and alarm on partial completion.

**What the self-destruct was for.** Tearing down cloud firewall rules and preventing billing for an orphaned VM.
**In the local variant both concerns vanish**, since the firewall rules are cloud objects that do not exist there and
there is no meter. The local equivalent is a physical disconnect or a wipe. **Note this differs from gate-0**, where
the same mechanism was also silently providing ephemerality that had to be rebuilt locally.

**Timeouts.** The original had two `until curl ...; do sleep ...; done` readiness loops with no deadline, so a hang
consumed the run without producing a verdict. Every wait needs a bound and a loud failure at that bound.

**Machine types.** `g4-standard-32` does not exist. Valid G4 shapes are `g4-standard-6/12/24/48/96/192/384`.
`g2-standard-32` does exist, so the original was likely a G2/G4 mix-up. This error appears in four documents in this
corpus.

**Cost.** Not a claim a client cares about. Track it internally per `../10-PREFLIGHT.md`, and never state a figure at
bundle-seal time, since billing export is not real-time.
