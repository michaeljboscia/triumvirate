# Raw peer output: gate-6-airgap-sanity.md unit 3 (lines 241-317)

Step 7 evidence upload via PGA, Step 8 teardown, Decision rule application, Cost accounting, What comes after.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Step 7 evidence upload is not defensible.** It uploads `/tmp/evidence/$RUN_ID` recursively to
`gs://pantheon-evidence/gate-6/` via `gsutil -m cp -r` **from inside the VM under test** (244-260). The uploading
identity is **the VM's attached service account, using in-VM credentials.** The excerpt does not constrain what that
identity can read locally, shows no least-privilege GCS permissions, **does not use a client-controlled bucket**, and
scopes the upload no further than the destination path. **No hash, manifest completeness check, or size check before
or after.**

**Could a file outside the evidence directory be uploaded?** Directly the command names only
`/tmp/evidence/$RUN_ID`, but **indirectly yes, via a symlink or planted content inside that tree, because there is no
validation boundary.**

**2. Step 8 does NOT have gate-0's `exit` defect.** The instance delete follows both firewall deletions and runs
normally (266-271). **But teardown is not transactional.** Failure between the two rule deletions leaves the VM
running with one rule removed; failure after both and before the instance delete leaves the VM behind with neither
the deny nor the PGA rule in place.

**3. The decision rule is still inconsistent and overclaims.** PASS is: `H-6.1: outbound packets <= 5`,
`H-6.2: 4/4 canonical tasks complete successfully`, `H-6.3: evidence bundle lands in GCS via PGA` (278-281). FAIL
branches cover leak, stall, and broken upload (285-297). **No inconclusive branch.** It does not match the rewritten
Rule B: **no independent packet-capture evidence requirement, no stated IPv6 coverage, no connected-baseline output
equivalence.**

**4. Cost accounting is stale.** `g4-standard-32` appears in both the manifest and the cost table (249, 305). Current
G4 shapes are `g4-standard-6/12/24/48/96/192/384`. **`g2-standard-32` exists, so this likely mixes G2 and G4.**

**5. Unsupported claims remain.** *"Sovereign claim validated. Ready to ship Pantheon Sovereign / Vault tier"* (283)
and *"audit-defensible"* (315) **are not supported by this evidence model:** PGA upload is unexamined exfiltration,
evidence is self-attested, IPv6 is absent, and the pass threshold contradicts the stated air-gap claim.

**6. Fails today:** any create path using `g4-standard-32`.

---

## GEMINI (strategic angle)

**1. Step 7 completely fails to answer the CISO objection.** Lines 258-259 are a standard `gsutil cp` to what appears
to be an internal bucket. **No attestation, no manifest hash, no proof of a client-controlled sink.** Required before
acceptance: cryptographic signatures of the payload, a verifiable manifest hash, and **IAM enforcement guaranteeing a
least-privilege write-only pipe to a bucket owned exclusively by the client.**

**2. The decision rule is unacceptable as a contract clause.** Line 279 permits *"outbound packets <= 5 (allowing for
incidental retry noise)."*

> **"'Incidental retry noise' is a loophole, not a contract clause. A vendor could easily exfiltrate keys within 5
> packets and claim it was an accident."**

The only acceptable contractual condition for an isolation claim is **zero unclassified, un-allowlisted packets.**

**3. What the self-destruct was FOR** (264-272): tearing down software firewalls (268-269) and preventing billing for
an orphaned VM (271). **On a local box that can be physically unplugged, the billing and resource-orphaning problem
disappears.** Replaced by a physical power disconnect or a bare-metal wipe.

**4. "What comes after" (313-317) is wrong about the next step.** It proceeds to soak/stress testing. **Passing the
product claim should instead force cryptographic sealing of the release artifact and generation of the compliance
attestation the client audits.**

**5. If this gate is the entire product, the only three things that convince a security team:**
1. **A strict zero-unclassified-packet capture** proving isolation.
2. **Cryptographic attestation** of the runtime and the evidence bundle.
3. **A verifiable, least-privilege, write-only pipeline to a client-controlled audit sink.**

Everything else (the dummy JSON, cost accounting, software firewall teardown) **becomes internal QA noise.**

---

## DEEPSEEK (adversarial logic angle)

Asked to quantify why a packet-count tolerance is the wrong control.

> On normal 1500-byte MTU paths a single packet carries roughly 1460 bytes of TCP payload. **Four packets is about
> 6 KB, enough for an SSH or TLS private key, a password-vault export, API tokens, a seed phrase, a small encrypted
> archive, or source and config snippets.**
>
> **Why a numeric tolerance is the wrong shape:** a packet count is **orthogonal to intent.** It models "noise" as a
> quantity, but an attacker can choose content that fits the budget, **so the threshold legitimizes arbitrary data as
> incidental.** "Noise" stays unfalsifiable unless every packet is attributable to an approved purpose.
>
> **The right shape:** default-deny egress with allowlisted destinations, protocols, and application-layer semantics;
> every packet must be traceable to a known permitted flow or be blocked and logged. **The only defensible threshold
> is zero packets that are not explicitly authorized, with those exceptions enumerated, constrained, and inspected,
> not counted.**

**This closes the argument quantitatively.** The document's own passing threshold leaves room for roughly 6 KB of
undeclared outbound data in a test whose entire purpose is proving nothing undeclared leaves. **A private key is
under 2 KB.**

**Three-peer convergence on the replacement:** not a smaller number, but a different kind of control. Enumerate and
inspect the permitted flows; everything else is zero.
