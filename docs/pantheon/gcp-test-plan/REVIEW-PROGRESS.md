# Review Progress: corpus remediation state

**THIS FILE EXISTS TO SURVIVE COMPACTION.** If you are a session that just lost context, read this file and
`/Users/michaelboscia/projects/triumvirate/docs/pantheon/GOAL-corpus-remediation.md`, then resume at the position marked
RESUME HERE. Do not re-review anything already logged below. Do not start over.

**Update this file after every section review, before moving to the next section.** Findings recorded only in
conversation context are lost at compaction. If it is not written here, it did not happen.

---

## RESUME HERE

**Current queue item:** 5 of 9 (`runbooks/gate-6-airgap-sanity.md`)
**Current section:** ALL 3 UNITS REVIEWED. Review of queue item 5 is COMPLETE.
**Next action:** REWRITE `gate-6-airgap-sanity.md`. Organizing principle from Gemini's distillation: three claims a client can check (zero-unclassified-packet capture, cryptographic attestation of runtime and bundle, write-only pipeline to a CLIENT-CONTROLLED sink), with operational detail demoted to an appendix. Split cloud restricted-egress validation from literal local air-gap. Move capture and adjudication outside the guest via an inline egress gateway, not a passive tap. Replace the packet tolerance with an allowlist at zero unclassified.

**Unit plan for queue item 5 (3 units):**
| Unit | Lines | Contents | Status |
|---|---|---|---|
| 1 | 1-107 | purpose, hypotheses, checklist, Step 1 | **DONE** |
| 2 | 108-240 | Steps 2-6, lockdown and capture analysis | **DONE** |
| 3 | 241-317 | Step 7 PGA upload, Step 8 teardown, decision rules, cost, next | **DONE** |

**Unit plan for queue item 4 (3 units):**
| Unit | Lines | Contents | Status |
|---|---|---|---|
| 1 | 1-111 | purpose, hypotheses, checklist, Steps 1-2 | **DONE** |
| 2 | 112-245 | Step 3 compose stack, Steps 4-5 tests | **DONE** |
| 3 | 246-314 | Step 6 evidence, Step 7 self-destruct, Step 8 vault, cost, what comes after | **DONE** |

**Carry into unit 3:** the `gcloud compute instances delete` at line 284 sits AFTER `exit`, so self-destruct never runs. Decision 9 is the Mac Studio purchase tied to a WWDC expectation that has since been overtaken by the M5 Ultra announcement.

**Unit plan for queue item 3 (3 units):**
| Unit | Lines | Contents | Status |
|---|---|---|---|
| 1 | 1-127 | framing, Decisions 1-3 (CapEx triggers) | **DONE** |
| 2 | 128-246 | Decisions 4, 5, 6, 7, 8 | **DONE** |
| 3 | 247-350 | Decisions 9, 10, rule application log, amendment protocol | **DONE** |

**Carry into unit 3:** Decision 10 is the auto OPEX-to-CAPEX trigger at $1000/mo for 2 months and was called the most dangerous line in the corpus.

**Unit plan for queue item 2 (4 units):**
| Unit | Lines | Contents | Status |
|---|---|---|---|
| 1 | 1-47 | header, design goals, directory structure | **DONE** |
| 2 | 48-321 | required file schemas | **DONE** |
| 3 | 322-390 | lifecycle, downstream consumers | **DONE** |
| 4 | 391-end | storage economics, retention, versioning, what this enables | **DONE** |

### OPERATING CONSTRAINT discovered 2026-08-25, obey it

**DeepSeek times out at the bridge's 180s ceiling on long prompts.** `TRIUMVIRATE_DAEMON_ASK_TIMEOUT_SECS` is read
client-side in `daemon-http` (lib.rs:437) by the MCP bridge process, so it cannot be raised without restarting the
session. Two long DeepSeek prompts failed; a short single-question prompt at `reasoning_effort: low` returned
immediately.

**Working pattern per section:**
1. Codex and Gemini get the full section by absolute path plus line range. They read from disk. Fire both in one message.
2. DeepSeek gets ONE focused question, `reasoning_effort: low` or `medium`, minimal pasted context. It cannot read files.
3. Append every peer's verbatim output to `review-raw/<doc>-<section>.md` immediately.
4. Update this file.
5. Commit.

Do not send DeepSeek a six-part question with a large pasted body. It will time out and the work is lost.

---

## QUEUE STATUS

| # | Document | Status | Commit |
|---|---|---|---|
| 0 | `HARDWARE_DECISION.md` + provenance | **DONE**, archived, TPS floor extracted into buy-vs-rent section 6 | `401fdde` |
| 1 | `gcp-test-plan/10-PREFLIGHT.md` | **REVIEWED + REWRITTEN** (11 sections, 113 findings) | see below |
| 2 | `gcp-test-plan/20-EVIDENCE-BUNDLE-SPEC.md` | **COMPLETE** (4 units, 3 peers, rewritten 444 to ~340 lines, verified) | `76a219d` |
| 3 | `gcp-test-plan/30-DECISION-RULES.md` | **COMPLETE** (3 units, 3 peers, 350 to 285 lines, verified) | `6e20292` |
| 4 | `runbooks/gate-0-plumbing.md` | **COMPLETE** (3 units, 3 peers, 314 to ~270 lines, verified) | `e675e92` |
| 5 | `runbooks/gate-6-airgap-sanity.md` | **REVIEWED**, 3 of 3 units. Rewrite next. | |
| 6 | `local-inference-buy-vs-rent.md` | partially touched (TPS floor added) | `401fdde` |
| 7 | `model-selection.md`, `graduated-gcp-validation-plan.md` | pending | |
| 8 | `runbooks/gate-1` through `gate-5`, `gate-7` | pending | |
| 9 | `twin-review-synthesis.md` | pending | |

**Section list for queue item 1 (`10-PREFLIGHT.md`), 11 sections:**

| Section | Lines | Reviewed |
|---|---|---|
| Phase 1: Project + billing + quota | 11-185 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 2: Network + storage | 186-272 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 3: Docker image pre-bake | 273-371 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 4: Model weights cached to GCS | 372-476 | **DONE** (Codex, Gemini, DeepSeek) |
| Phase 5: PD snapshots | 477-544 | **DONE** (Codex, Gemini, DeepSeek). VERDICT CORRECTED, see findings |
| Phase 6: Custom VM images | 545-642 | **DONE**. VERDICT: DELETE PHASE |
| Phase 7: Fixtures + Pythia seed | 643-676 | **DONE**. See THE CENTRAL FINDING |
| Phase 8: Tooling validation | 677-726 | **DONE**. Validates nothing, see P8-C1 |
| Preflight completion checklist | 727-755 | **DONE** |
| Cost accounting | 756-773 | **DONE** |
| What comes next | 774-end | **DONE** |

---

## FINDINGS LOG

### `gate-6-airgap-sanity.md` unit 3 (lines 241-317): upload, teardown, decision rule, claims

Raw output: `review-raw/gate-6-unit-3.md`. All three peers. **Completes the review of queue item 5.**

#### THE FINDING THAT ENDS THE THRESHOLD ARGUMENT

**G6-C8. The passing threshold leaves room to exfiltrate a private key, and the rationale for it is unfalsifiable.**

Line 279 passes at `outbound packets <= 5`, rationale *"allowing for incidental retry noise."*

DeepSeek quantified it: at a 1500-byte MTU a packet carries roughly 1460 bytes of payload, so **four packets is about
6 KB. An SSH or TLS private key is under 2 KB.** Also within budget: a password-vault export, API tokens, a seed
phrase, a small encrypted archive.

Gemini as the client's CISO: **"'Incidental retry noise' is a loophole, not a contract clause. A vendor could easily
exfiltrate keys within 5 packets and claim it was an accident."**

**The general principle, which is why the fix is not a smaller number:** a packet count is **orthogonal to intent.**
It models noise as a quantity, so an attacker simply chooses content that fits the budget, and the threshold
**legitimizes arbitrary data as incidental.** Noise stays unfalsifiable unless every packet is attributable to an
approved purpose.

**The right shape of control:** default-deny egress with allowlisted destinations, protocols, and application-layer
semantics. **Every packet is traceable to a known permitted flow, or it is blocked and logged. Zero unauthorized
packets, with exceptions enumerated, constrained, and inspected rather than counted.**

#### CRITICAL

**G6-C9. Step 7 is a plain recursive copy from inside the VM, using the VM's own credentials, to OUR bucket.**
Lines 244-260. The uploading identity is the VM's attached service account. **No least-privilege scoping, no
client-controlled sink, no hash or manifest completeness check before or after, and no size check.**

Codex on the boundary: the command names only the evidence directory, **but a symlink or planted content inside that
tree would be followed, because no validation boundary exists.**

Gemini as CISO: *"No attestation, no manifest hash, no proof of a client-controlled sink."* This is the unit-1
objection made concrete, **and Step 7 does not answer it in any respect.**

**G6-C10. "Sovereign claim validated. Ready to ship" and "audit-defensible" are unsupported by this evidence model.**
Lines 283 and 315. PGA upload is unexamined exfiltration, evidence is self-attested, IPv6 is absent, and the pass
threshold contradicts the document's own air-gap claim. **These are the two sentences a prospect would quote back.**

#### HIGH

**G6-H9. Teardown is not transactional.** Lines 266-271. **Good news first: unlike gate-0, the instance delete is not
stranded after an `exit` and does run.** But a failure between the two firewall-rule deletions leaves the VM running
with one rule removed, and a failure after both leaves the VM with neither the deny nor the PGA rule in place.
**Neither intermediate state is safe, and nothing detects them.** *(Codex)*

**G6-H10. The PASS condition has no inconclusive branch** and does not match the rewritten Rule B: **no independent
packet-capture requirement, no stated IPv6 coverage, no connected-baseline equivalence.** The gate that adjudicates
the product claim is weaker than the rule that governs the claim. *(Codex)*

**G6-H11. `g4-standard-32` appears again**, in both the manifest and the cost table (249, 305). Codex notes
`g2-standard-32` does exist, **so this is likely a G2/G4 mix-up** rather than an invented name. Fourth document
containing it. *(Codex)*

#### THE DISTILLATION (this decides the rewrite's shape)

Asked what three things actually convince a security team if this gate is the whole product, Gemini:

1. **A strict zero-unclassified-packet capture** proving isolation.
2. **Cryptographic attestation** of the runtime and the evidence bundle.
3. **A verifiable, least-privilege, write-only pipeline to a client-controlled audit sink.**

**Everything else, including the cost accounting and the software firewall teardown, is internal QA noise.** That is
the right organizing principle for the rewrite: **three claims a client can check, and an appendix of operational
detail that exists for us rather than for them.**

#### WHAT THE SELF-DESTRUCT WAS FOR (standing rule)

Lines 264-272. **What it was for:** tearing down the software firewall rules and preventing billing for an orphaned
VM. **Does the problem persist?** On a local box that can be physically unplugged, **the billing and cloud-orphaning
half disappears.** What replaces it is a physical disconnect or a wipe. **Note this differs from gate-0**, where the
same mechanism was also providing ephemerality that had to be reconstructed locally; here the firewall rules are
cloud objects that simply do not exist in the local variant. *(Gemini)*

#### WHAT SHOULD HAPPEN ON A PASS

Not "proceed to soak testing" (317). Gemini: **passing the product claim should force cryptographic sealing of the
release artifact and generation of the compliance attestation the client audits.** Under Rule A the gate must push
forward into producing the client-facing artifact, not into more internal testing.

---

### `gate-6-airgap-sanity.md` unit 2 (lines 108-240): lockdown, execution, capture analysis

Raw output: `review-raw/gate-6-unit-2.md`. All three peers.

#### THE STRUCTURAL FINDING (three-peer convergence, and it decides the rewrite)

**G6-C5. The system captures, stores, and adjudicates the evidence of its own isolation. All four roles are the same
machine.**

`tcpdump` runs inside the VM (129), writes to the VM's own disk, and Step 6 reads that same file back inside the VM
and writes the verdict (221-233). **Observer, observed, evidence store, and adjudicator are one entity.**

Gemini: *"a malicious payload in Step 4/5 could simply kill the capture, exfiltrate data, and forge the pcap before
Step 6 ever runs."*

DeepSeek states the class precisely: **this is self-attestation, or trusted-host audit, not independent proof.** It
holds under a threat model where the OS, kernel, firewall, tcpdump, disk, and verdict code are all trusted and **only
the workload is potentially malicious.** It fails entirely if the host itself may be compromised. *"The evidence has
no independent witness. This is like a defendant writing and signing their own alibi."*

**This is the same defect as the evidence-bundle independence problem (E1-S2), one level deeper: the capture that
adjudicates the product claim is produced by the thing it evaluates.**

**Fix, and the nuance matters more than the headline.** Move the observer outside the trust boundary. **But a passive
tap or mirror is not enough:** it only sees packets that reached the wire, so **blocked attempts are invisible to it,
and blocked attempts are the most persuasive half of the evidence.** The external component must be an **inline
egress gateway that is itself the deny-all enforcement point**, so it can log what it refused as well as prove nothing
escaped.

**Cost, stated honestly:** extra infrastructure, a topology change, confirming no alternate egress path exists, added
latency, a new single point of failure, and the external observer itself needs securing and auditing.

#### CRITICAL

**G6-C6. The threshold is `-lt 5`, and it counts the wrong thing.**
Line 233: `"verdict": "$([ $OUTBOUND_COUNT -lt 5 ] && echo PASS || echo FAIL)"`. So it passes at four or fewer, a
fourth distinct number in a document that elsewhere says zero. Worse:
- **It counts decoded lines, not verified egress** (221-224).
- **It does not distinguish blocked attempts from successful traffic**, which is the entire distinction the claim
  rests on.
- Line 227 calls traffic outside the PGA endpoints a leak, **but the count includes allowed PGA traffic, with no
  destination-based adjudication at all.**
*(Codex)*

**G6-C7. IPv6 is unprotected and uninspected.**
The deny rule covers `0.0.0.0/0` only (141-148), so **IPv6 egress is not denied**, and the capture filter is
IPv4-oriented so it is not observed either. **An entire address family is outside both the control and the
measurement.** *(Codex)*

#### HIGH

**G6-H5. The capture starts after the stack is prepared, so the preload window is invisible by construction.**
Codex enumerated what enters the machine before capture begins (108-125): three container images, the canonical task
fixtures from GCS, and the Pythia corpus, plus Artifact Registry auth.

Gemini: preparing before lockdown is **a practical necessity** unless everything is pre-baked, **but it destroys a
literal air-gap claim.** The honest response is to **rename the claim to runtime isolation and explicitly bound the
trusted pre-provisioning window** rather than leave the gap unmentioned.

**G6-H6. tcpdump may truncate payloads and its exclusions are unjustified.**
`sudo tcpdump -i any -w ... 'not (dst net 10.128.0.0/20 or dst net 127.0.0.0/8)'` (129-130). **No `-s 0`**, so the
default snap length may truncate. Interface `any`, none named or validated. **Neither excluded range is justified
anywhere in the document**, and an unexplained exclusion in an evidence artifact is exactly what an auditor probes.
*(Codex)*

**G6-H7. Two readiness loops can hang forever.**
`until curl ...; do sleep ...; done` at 181 and 189, with no deadline. Step 5 has `--timeout-per-task=300` but no
overall bound. **A hang here consumes the run without producing a verdict.** *(Codex)*

**G6-H8. The swarm summary is printed and never enforced.**
Step 5 prints `swarm-summary.json` (210) and the runbook never parses it, so **the pass condition is a human glancing
at output.** Gemini: it must assert the swarm's metrics match a non-air-gapped baseline and produce the evaluated task
outputs, not merely that nothing crashed. *(Both, consistent with Rule B)*

#### THE FIREWALL, WHICH IS CORRECT AS FAR AS IT GOES

Recorded because it is one of the few mechanisms in this corpus that works as intended. Deny egress `0.0.0.0/0` at
priority 100, allow PGA `199.36.153.8/30,199.36.153.4/30` tcp:443 at priority 50. **Priority ordering does what the
author intended** and the implicit allow-egress is correctly overridden for IPv4. The gap is IPv6 (G6-C7), not the
logic. *(Codex)*

#### THE ALLOWANCE, REPLACED (standing rule)

**What `<= 5` was for:** unavoidable OS-level noise, DHCP, ARP, NTP, metadata lookups, or telemetry firing before the
firewall clamps.

**Replacement:** an explicit protocol and destination allowlist, **zero unclassified packets**, each named exception
accounted for by destination and protocol. Gemini as auditor: *"a bare packet count is indefensible."* It says only
"some packets left and we decided that was fine."

---

### `gate-6-airgap-sanity.md` unit 1 (lines 1-107): purpose, hypotheses, checklist, Step 1

Raw output: `review-raw/gate-6-unit-1.md`. All three peers. **This document is the product claim.**

#### THE MOST CONSEQUENTIAL FINDING IN THE ENTIRE REVIEW

**G6-C1. The gate's evidence channel is an exfiltration channel, by design, and the document never acknowledges it.**

Gemini, reading as a client CISO:

> "You intentionally punched a hole for Private Google Access to upload evidence bundles. **How do you
> cryptographically or programmatically guarantee that those payloads sent to GCS do not exfiltrate our proprietary
> context, weights, or PII through that allowed channel?**"

The gate proves isolation by denying egress, then deliberately keeps one egress path open to upload the proof. **The
proof channel is the hole.** Nothing in the document constrains what flows through it.

DeepSeek: **answerable, but fatal if the answer is "trust us."** The fix is to make the allowed path a
**constrained, auditable, write-only pipe:**

1. **Point it at a CLIENT-CONTROLLED bucket**, not ours, with a policy permitting only `PutObject` to a fixed evidence
   prefix. **This inverts who holds the evidence and removes the vendor from the trust path**, which is the single
   highest-leverage change available here.
2. **Least-privilege uploader:** a separate identity that can read only the evidence directory, never weights,
   databases, source, or client data, enforcing an allowlist of expected filenames, types, and sizes, aborting on
   anything unexpected.
3. **Manifest and hashes computed before upload**, so smuggled data shows up as an unexpected object and breaks the
   manifest.
4. **Remote attestation** binding the uploader to a reviewed image.

**Client-verifiable rather than vendor-asserted:** the network path and bucket policy (client audits flow logs), the
uploaded object list against the manifest (client checks their own bucket), and the attestation report.

**The one thing that cannot be asserted is that the uploader has no access to sensitive data.** That needs code or
image review, or attestation. **Put this in the rewrite as an explicit section; a security team will ask it first.**

#### CRITICAL

**G6-C2. The document contains THREE mutually inconsistent thresholds for the same claim.**
- Line 3: *"Any packet leaving the VM = fail"* and *"ZERO outbound traffic."*
- Line 45: *"Zero -> PASS, 1-10 attempts -> investigate, > 10 -> fundamental architectural leak."*
- Elsewhere (unit 2 range): passes at `outbound packets <= 5`.

**Codex: the load-bearing number in this range is zero, and the diluted band at line 45 contradicts it.** A gate with
three thresholds has none. It also measures *"attempts that reach the firewall"* (43) rather than all packets, which
is a narrower thing than either claim.

**G6-C3. Step 1 applies no firewall rules at all.**
Lines 92-105 provision with `--no-address` and **create no deny or allow egress rules whatsoever.** The gate's central
mechanism is absent from the step that claims to establish it. Default VPC egress may remain open unless rules exist
outside this range. *(Codex)*

**G6-C4. Four egress paths are unaddressed:** IPv6 (no `::/0` denial), the metadata server at `169.254.169.254`
(neither blocked nor monitored), DNS resolution, and NTP. *(Codex)*

#### HIGH

**G6-H1. The opening claim is false on the document's own terms.**
Line 14 says Pantheon *"does not attempt any outbound network connection"* while lines 59-60 and 71 test outbound
GCS uploads via PGA. **A prospect who reads the whole document finds the contradiction themselves.**

Gemini's replacement, which is both compelling and true: *"When deployed in a sovereign environment, Pantheon operates
completely isolated from the public internet, restricting all egress exclusively to customer-authorized private
endpoints."*

**G6-H2. H-6.2 checks completion, not correctness. Fourth instance.**
Lines 52-56 assert tasks complete and pass eval without defining the eval's rigor. Gemini's wording: *"All tasks pass
evals that strictly require local retrieval and inference to succeed, proving the system did not silently fall back,
skip steps, or return validly-shaped empty results."* **Rule B in the rewritten decision rules also now requires a
connected-baseline parity check, which this hypothesis does not have.**

**G6-H3. The capture would miss most of what matters.**
tcpdump appears only in a prediction (43), the firewall is applied *"after stack is up"* (91), and **no interface is
specified.** It therefore misses provisioning and startup traffic before lockdown, anything on an uncaptured
interface, metadata/DNS/NTP paths, and PGA traffic unless filters explicitly include it. **Start capture before
lockdown, name the interfaces, and cover both address families.** *(Codex)*

**G6-H4. The exclusion list omits exactly what the claim depends on.**
"What it does NOT validate" (31-35) excludes hardware representativeness and drift, **but not IPv6 coverage, metadata
server access, DNS and NTP paths, packet-capture scope, or output correctness.** Those are the real limits, and a
client security team will find them whether or not the document names them. *(Codex)*

#### THE `<= 5 PACKETS` ALLOWANCE (standing rule applied)

**What it was for:** unavoidable OS-level noise, DHCP, ARP, NTP, metadata lookups, or telemetry firing before the
firewall clamps down. **Does the problem persist?** Yes; OS defaults still generate it.

**But a bare tolerance number is indefensible to an auditor,** since it says only "some packets left and we decided
that was fine." **Replacement: an explicit protocol and destination allowlist. Zero unclassified packets. Named
exceptions only, each accounted for by destination and protocol.** That is strictly stronger and easier to defend
than a count. *(Gemini)*

#### STRUCTURAL

**G6-S1. Split into two documents rather than one with two modes.**
Under the owner's terminology ruling, the GCP test is honestly **cloud restricted-egress validation** and only the
physically disconnectable local box supports a literal **air-gap** claim. Gemini: a runbook must be a linear
executable script, and combining modes forces branching between gcloud provisioning and local scripts, and between
PGA rule verification and physical isolation. **One document with two modes introduces execution drift and dilutes
the audit trail.**

**G6-S2. `g4-standard-32` (line 93) is not a valid machine type.** Third document containing this error. *(Codex)*

---

### `gate-0-plumbing.md` unit 3 (lines 246-314): evidence, teardown, vault, cost, next steps

Raw output: `review-raw/gate-0-unit-3.md`. All three peers. **Completes the review of queue item 4.**

#### THE FIRST-RUN FAILURE PREDICTION

**G0-F1. It will fail at `/opt/pantheon-harness/`, immediately, with file-not-found.**
Lines 250, 254, 273, 278 call Python scripts at that absolute path. **It is native to a custom GCP image template
that was never built, and the local box has no such directory.** Gemini's answer to "what goes wrong first," and it is
concrete enough to fix before running rather than discovering at 2am.

Second most likely: **a hang with no timeout**, leaving ports squatted and state contaminated, since nothing bounds
the run and Step 7 never executes (see G0-C5).

#### CRITICAL

**G0-C5. The self-destruct is dead code, confirmed in context.**
Line 285 runs `exit`; line 286 never executes. **On the original GCP target the VM would simply stay alive** after the
logs, cost report, and note were uploaded. Nothing in the block saves it; only an external TTL or manual cleanup
would have. **This was the third of six advertised spend-control layers to be found non-functional.** *(Codex)*

**G0-C6. Step 6 is named "evidence bundle emission" and is not the last writer to the bundle.**
Lines 250-260 generate a manifest and summary and upload the directory, reading as finalization. **Then Step 7 writes
more objects into the same bundle** (logs, cost report, note) at 269-282. No content-addressing, no sentinel, no
finalize. Validation is `gsutil ls -r` (263), which lists objects and validates neither schema, checksums, required
files, nor completeness.

DeepSeek on what that reveals: the runbook was **written incrementally**, Step 6 authored as the terminal step and
Step 7 appended later without revisiting Step 6's name or the end-to-end data flow. *"The name 'emission' was chosen
from intention, not from actual behavior."*

**The discipline, and it generalizes: name steps by the invariant they guarantee, not by their role in a draft
narrative. Only the step that leaves the artifact in its terminal state may be called emission.**

**This is the corpus-wide tense finding in a different grammatical category.** The tense rule governs verbs; this
governs step names. Both are labels asserting something the procedure does not do. **Check every step name implying
terminality (emit, finalize, complete, publish, verify) and confirm nothing after it touches what that step claims to
finish.**

#### HIGH

**G0-H9. Step 8's "verify" can pass against a partial bundle.**
Line 292 lists the prefix. Any objects existing satisfies it. **It validates presence, not completeness**, which is
exactly the gap the sentinel exists to close. *(Codex)*

**G0-H10. "What comes after" is passive documentation, which under Rule A licenses stopping.**
Lines 312-314 state that a PASS leads to the next gate. Gemini: **to force advancement, passing must output the
literal execution command for the next stage, or trigger it.** A sentence describing what should happen next is not a
forcing function. Line 312 also calls the next gate the "first real GPU burn," which no longer holds locally, and
line 314's "cheapest debug available" is unsupported rhetoric. *(Both)*

**G0-H11. Cost accounting states a dollar total that is now meaningless.**
Line 304 assumes `e2-standard-4` Spot pricing and line 306 totals `~$0.10`, for a run on a machine we own.

**What it was for** (standing rule): preventing budget drain. **Does the problem persist locally?** The financial form
does not; the **constraint** form does. **Replacement (Gemini): resource accounting.** Track local disk consumed by
artifacts and execution time (operator waiting, process locking). Those are the scarce resources now. If anything
still uploads to cloud storage, use `cost_status: pending_billing_export` rather than a number. *(Both)*

#### THE REWRITTEN STEP 7 (both peers converge, ordered)

1. **Capture telemetry first:** `docker compose ... logs > ...` before anything is destroyed.
2. **Eradicate state:** `docker compose ... down -v --remove-orphans`. This is the missing H-0.4 teardown.
3. **Archive evidence:** move `/tmp/evidence/$RUN_ID` to permanent local storage.
4. **Scrub temps:** remove the staged evidence directory and the temp compose and config files.
5. Delete both the `exit` (285) and the `gcloud compute instances delete` (286).

Codex's additions: free and verify the bound ports; remove named volumes **only if test-scoped**; revoke or delete
temporary credentials and token files. **Explicitly avoid deleting shared Docker resources or long-lived local
credentials**, which is the failure mode a blunt cleanup introduces.

**The whole teardown must run unconditionally, on any exit path**, which is the point of a trap rather than a final
step. That plus a runner timeout is what replaces the ephemerality the VM used to provide for free.

#### MEDIUM

**G0-M6. The Obsidian vault step does not belong in this runbook.**
Lines 289-296 assume `~/Documents/pantheon-vault` and therefore a specific operator machine. **What it was for:**
integrating run metadata into a personal knowledge base for historical search. **The purpose survives; the placement
does not.** Extract to a global post-run hook or reporting script. Gemini: *"Tying PKM git commits to a specific test
stage creates brittle coupling."* Consistent with the evidence-spec decision to move `obsidian-note.md` to an
internal sidecar. *(Both)*

---

### `gate-0-plumbing.md` unit 2 (lines 112-245): compose stack, health check, dispatch test

Raw output: `review-raw/gate-0-unit-2.md`. All three peers.

#### THE RESOLVED DISAGREEMENT (this one changes the test design)

**G0-R1. Codex and Gemini contradicted each other on what the dispatch test should assert. DeepSeek drew the boundary
and both turn out right about different things.**

- **Codex:** `tasks_completed: 5, tasks_errored: 0` (239-242) is too weak; an empty or malformed result counts as
  completed. Assert output correctness.
- **Gemini:** a plumbing test *"must explicitly avoid asserting latency or output semantics"*, because the inference
  is mocked and asserting on mock output tests the mock.

**DeepSeek's rule:** assert that each task's **routing envelope** (identity, headers, source, destination, trace
context, deterministic control fields) arrives intact and lands at the right stage; **do not** assert the semantics of
the mock's output payload.

- **In scope:** task `#3` entered the mocked inference stage with request ID `req-3`, and the orchestrator routed the
  response back with the same correlation ID, **regardless of whether the body is `{"ok":true}` or `"malformed"`**.
- **Out of scope:** asserting the mock returned a correct answer, which tests the mock's hardcoded behavior.

**So the correct assertion is neither "5 completed" nor "the answer was right." It is: every correlation ID
round-tripped, each task reached the stage it should have, and the envelope was not corrupted.** Strictly stronger
than the current test, strictly narrower than checking answers. **This is the design for the rewritten Step 5.**

**G0-R2. Line 241 asserts `round_trip_median_ms: 45`, a performance claim inside a plumbing test.**
Gemini caught it. A latency assertion against a mock measures the mock and the local machine's load at that moment.
**Remove it.** Latency belongs in the sizing sweep, where it is measured against real inference. *(Gemini)*

#### CRITICAL

**G0-C3. Step 3 cannot run: both compose files are `# [paste content from above]` placeholders.**
Lines 175-182. `docker compose up` at 185 starts nothing. Previously flagged; confirmed here in context. *(Codex)*

**G0-C4. Fixed host ports with no collision guard, on a machine that stays up.**
Ports `4222`, `8222`, `8000`, `7788` bound at 123, 133, 147, with **no compose project name, no preflight port check,
and no cleanup.** On a persistent box these can already be held by NATS, an Ollama-style service, a dev server, or a
previous Triumvirate run. **This is the concrete form of the ephemerality loss from G0-C1.** *(Codex)*

#### HIGH

**G0-H5. Three state-contamination sites, enumerated.** *(Gemini)*
- `/tmp/docker-compose.gate-0.yml` and `/tmp/config/gate-0.toml` persist (175-182).
- `docker compose up -d` leaves containers, networks, and port mappings running indefinitely (185).
- `/tmp/evidence/$RUN_ID` persists permanently, and **if `RUN_ID` generation fails or collides, evidence is
  contaminated by a prior run** (211, 227-229).

**There is no teardown step anywhere in the runbook.** Without one, state contamination is guaranteed rather than
possible.

**G0-H6. The time bound must survive the move to local, for a different reason than it existed.**
It was billing protection on GCP. Locally it stops **a hung process indefinitely squatting ports 4222, 7788, and 8000
and blocking every future run.** Replace with a strict runner timeout plus an unconditional teardown hook that fires
regardless of exit code. *(Gemini, extending G0-C1)*

**G0-H7. H-0.1 defines "healthy" as an HTTP 200 and nothing more.**
Lines 196-202 and evidence at 208-210 record only OK/FAIL from curls against host ports. **A service can answer HTTP
while being unable to reach NATS or dispatch anything.** *(Codex)*

**G0-H8. Nothing bounds the dispatch.** Healthchecks have bounded probes, but orchestration uses a blind `sleep 15`
(188) rather than waiting deterministically, and the dispatch at 218-227 **has no timeout at all, so a blocked harness
hangs the run.** *(Codex)*

#### MEDIUM

**G0-M3. Healthchecks assume tools that may not be in the images:** `wget` in the NATS image, `curl` in the harness
and Triumvirate images (125, 135, 152). **A healthcheck can fail for a reason unrelated to health.** *(Codex)*

**G0-M4. `depends_on` (148-150) waits on container healthchecks, not application readiness**, and is undermined
anyway by the `sleep 15`. *(Codex)*

**G0-M5. Image references still point at Artifact Registry** (121, 131, 141, 223). Local execution needs local tags
or a registry mapping. *(Codex)*

#### WHAT A PASSING GATE 0 LICENSES

Worth stating explicitly in the rewrite, because the corpus has a habit of overclaiming. Gemini: it proves **Docker
networking, NATS messaging, and Triumvirate configuration communicate.** It licenses **zero** confidence in GPU
allocation, CUDA drivers, model loading, or real vLLM stability. **Mocking is the right call for this gate** (it is
what isolates the variable), but it necessarily hides model loading, GPU/runtime compatibility, request schema
differences, streaming, memory pressure, and real inference failure, which are the things most likely to break next.

---

### `gate-0-plumbing.md` unit 1 (lines 1-111): purpose, hypotheses, checklist, Steps 1-2

Raw output: `review-raw/gate-0-unit-1.md`. All three peers.

#### THE FINDING THAT VALIDATES THE STANDING RULE

**G0-C1. The VM self-destruct was doing two jobs. Only one was written down, and only that one dies with the move to
local. TWO-PEER CONVERGENCE.**

Lines 84-85 set `--max-run-duration=45m` and `--instance-termination-action=DELETE`, documented as cost control. Move
to a box you own and the cost rationale evaporates, so the mechanism looks obviously removable.

**It was also enforcing ephemerality**, and that job survives the move intact. Gemini named the local symptoms: hung
processes, port collisions, dangling Docker volumes, disk exhaustion, all of which **contaminate the next gate**
because the machine no longer disappears between runs. DeepSeek added a third purpose neither of us had:
**credential and secret hygiene.** A long-lived local box accumulates auth state a disposable VM discards.

DeepSeek's general case, which is the owner's standing rule derived independently:

> "Unwritten secondary purposes accumulate because mechanisms solve whatever problem existed at the time, but only the
> headline reason gets documented. When the headline rationale disappears, people assume the mechanism is obsolete.
> **The unwritten job only becomes visible at removal time.** Treat removal as a design decision, not a cleanup task."

**Replacement:** a local timeout wrapper (`timeout 45m ./run-gate-0.sh`) with a teardown trap
(`docker compose down -v`) that fires **regardless of exit code**, plus explicit credential hygiene between runs.

**G0-C2. H-0.2 checks completion, not correctness. Third instance of this defect.**
Lines 43-45: "5/5 tasks complete round-trip." Codex: *"'complete' can pass bad output unless structured result
correctness is defined."* Same shape as the isolation gate (D2-C1) and the metrics template (E2-C1). **A canned task
that returns an empty-but-well-formed result passes.**

#### HIGH

**G0-H1. Missing hypothesis: H-0.4, clean teardown.**
The three hypotheses cover startup, execution, and evidence emission. **On a persistent machine, tearing down cleanly
is a testable property and nothing tests it.** Gemini: on a VM the machine disappears, so this was free; locally it
is not. *(Gemini)*

**G0-H2. The exit condition licenses stopping rather than forcing advance.**
Currently a PASS means "it worked." Under the rebuilt Rule A the exit condition should require a successful teardown
**and** mandate proceeding, since failure-to-advance is the mode this gate historically enabled. *(Gemini,
consistent with D2-H2)*

**G0-H3. Step 2 verifies almost nothing.**
Lines 100-111. `docker ps` (107) checks daemon access, not the required images, compose file, NATS, Triumvirate, mock
vLLM, ports, or GPU. `mkdir -p /tmp/evidence/$RUN_ID` (109) is **effectively non-failing** and proves no evidence
contract. `gcloud auth configure-docker` (108) is registry setup, not verification. **A verification step composed of
checks that cannot fail is the checklist problem in miniature.** *(Codex)*

**G0-H4. The scope boundary excludes something the gate's own claim depends on.**
"What it does NOT test" (24-32) reasonably excludes real inference, worker pools, and GPU scheduling. **But it also
leaves out output correctness and evidence completion semantics, while the gate claims end-to-end dispatch and
evidence validity (3, 22).** The exclusion list and the claim contradict each other. *(Codex)*

#### THE PURPOSE QUESTION (standing rule applied to the whole gate)

**What is this gate FOR now that the substrate is a machine we own?** The original rationale was proving orchestration
works before spending GPU dollars, and that rationale is gone.

Gemini's answer, which is correct and should open the rewritten document: **isolating variables.** Go straight to
real models and a timeout is ambiguous between a NATS failure and a vLLM OOM. The gate exists so that when the next
stage breaks, the plumbing is already known good. **That purpose is substrate-independent, which is why the gate
survives the move while most of its steps do not.**

#### CHECKLIST AUDIT (lines 55-64)

| Item | Status |
|---|---|
| `10-PREFLIGHT.md` complete | EXISTS as a document; never executed |
| `pantheon-orchestrator-v1` VM image | DOES NOT EXIST, GCP-only concept |
| `pantheon-triumvirate:main` | DOES NOT EXIST |
| `pantheon-test-harness:main` | DOES NOT EXIST |
| `pantheon-nats:2.10` | DOES NOT EXIST |
| `pantheon-vllm-cpu:v0.6.5` | DOES NOT EXIST (and the upstream base returns 404) |
| "No other Pantheon VMs live" | obsolete locally |
| GCS evidence bucket writable | DOES NOT EXIST, obsolete if evidence is local-first |
| **The Lenovo itself** | **EXISTS and is not mentioned anywhere in the checklist** |

**Five of eight items reference artifacts that do not exist, and the one machine that does exist is absent.** *(Codex)*

#### MEDIUM

**G0-M1. Step 1 is almost entirely GCP ceremony.** Only `RUN_ID` naming (72) and the image-source concept (73)
survive translation. Everything from 70-94 is provisioning that has no local analogue. **What a local run needs and
this does not provide:** verification of `ssh lenovo`, Docker, GPU identity and compute capability, VRAM, cores, RAM,
disk space, image availability, a cleanup guard, and a run cap. *(Codex)*

**G0-M2. H-0.3's threshold is by reference only** to the evidence spec (51), and does not require the `COMPLETE`
sentinel that the rewritten spec now mandates be written last. Update the reference. *(Codex)*

---

### `30-DECISION-RULES.md` unit 3 (lines 247-350): Decisions 9-10, logs, amendment protocol

Raw output: `review-raw/30-DECISION-RULES-unit-3.md`. All three peers. **Completes the review of queue item 3.**

#### THE BEST FINDING IN THIS DOCUMENT

**D3-B1. Decision 10 is the budget-bleed guard wearing the wrong clothes. Invert it.**

Unit 2 found that nothing in the corpus guards against budget bleed. Decision 10 already contains exactly the
machinery that gap needs: a spend threshold (`> $1000/mo for 2 consecutive months`, line 288), a
sustained-not-bursty condition (289), and break-even utilization hours by card class (291-293).

It just points the wrong way. As written it says *spend crossed the threshold, therefore buy hardware*, which uses
high rent as an argument for CapEx and directly subverts rent-first.

**Inverted, it becomes the missing rule:** *spend crossed the threshold, therefore STOP and justify continuing.* The
justification options are optimize the architecture, commit to a reserved instance, or shut the workload down.
Gemini: *"The inversion works perfectly and is not too clever. Renting is the destination."*

**This is the cleanest outcome of the whole review: the most dangerous line in the corpus, rotated 180 degrees,
becomes the control the corpus was missing.**

Codex's caveat on the break-even numbers: they assume purchase price, usable life, utilization, cloud hourly rate,
power/support/ops overhead, and workload equivalence, **none of which are stated, so the thresholds are not auditable
as written.** They survive translation only if rewritten around the **reserved-instance commitment delta**: on-demand
cost at observed hours versus reserved cost plus lock-in risk. Recompute before adopting the hour figures.

#### CRITICAL

**D3-C1. Decision 10 can force a purchase with no performance gate whatsoever.**
Codex: the only real conflict with the production floor in this range is that **Decision 10's trigger is purely
financial.** Nothing requires the hardware to clear 15 tok/s/stream at 4-way batch before money is committed. A
spend threshold alone can authorize buying something that cannot do the job. *(Codex)*

#### HIGH

**D3-H1. The amendment protocol is documentary, not binding. Gemini reversed her own unit-1 position, and DeepSeek
explains why both versions were partly right.**

Unit 1 (Gemini): the friction of writing down a bad excuse breaks the motivated-reasoning cycle.
Unit 3 (Gemini): *"Journaling a rationalization does not stop motivated reasoning; it just records it."*

DeepSeek resolves it: the first version *"works for people with enough integrity that seeing a bad excuse in writing
shames them into honesty, but that is a personality trait, not a mechanism."* The second holds **whenever the same
person controls the threshold, the reasoning, and the application, and faces no external cost.** That is exactly the
situation here. **A log records drift; it does not stop it. Only a veto from outside the motivated mind can.**

**Practical fix for a solo operator**, who cannot supply his own external veto:
1. **Amendments apply only to evidence collected after the change** (DeepSeek's minimum), never retroactively.
2. **A cooldown before an amendment takes effect** (Gemini proposed 72 hours).
3. **Peer review as the actual separation of powers.** The one genuinely external reviewer already available is the
   twin agents. An amendment reviewed by Codex or Gemini before taking effect is real, and costs almost nothing.

Codex adds what the protocol is missing structurally: immutable before/after text, amendment author, evidence IDs,
an explicit replacement rule, and a required statement of what the old rule was for and what replaces it. **That last
item is the owner's standing rule, and it belongs in the protocol itself rather than only in this review.**

**D3-H2. The rule application log is empty after four months, and that is the finding.**
Lines 310-325 are a sample object, not a real application. Gemini: *"An empty log after four months proves the
mechanism was a bureaucratic fantasy. If the tool is too heavy to pick up, it gets bypassed entirely."* Replace the
hand-written JSON requirement with something light enough to actually use: a ledger entry or a git commit.

**D3-H3. "What this document enables" (342-350) overclaims, same as every other closing section in this corpus.**
Line 344 is false because the log is unpopulated. Line 346's "auditable decision trail" does not exist. **Line 348 is
worse than aspirational: it cites an RTX Pro 6000 CapEx trigger as evidence of "clean business defensibility," and
that trigger is being cut for violating policy.** *(Codex and Gemini)*

#### DECISION 9, AND A NOTE THAT THE MACHINERY WORKED

**D3-D1. Applied mechanically, Decision 9 says DO NOT BUY, and that is the correct answer.**
Worth recording because it is the only place in this review where a pre-committed rule was applied as designed and
produced the right result. A 512GB M5 Ultra was announced 2026-08-25, which might look like a trigger. But the 256GB
purchase requires **all four** predicates at lines 256-259 (including a paid engagement and a friction log), and the
512GB path requires either sovereign/405B justification or 256GB being unavailable (261). Since 256GB exists, **the
fallback at 270-272 is now obsolete and the rule correctly declines to fire.** *(Codex)*

The stale `~$12K` figure for 512GB should be removed; Apple has not published that price.

**Replacement (Gemini):** rented Apple metal exists (AWS `mac-m3ultra.metal`, 256GB today). Replace the acquisition
triggers at 255-264 with **a mandate to rent first**, unlocking CapEx only if rental proves structurally unviable for
a named reason such as latency or an MLX-specific bottleneck. This matches the treatment already given to the same
question in `fast-vm-startup-strategies.md`.

---

### `30-DECISION-RULES.md` unit 2 (lines 128-246): Decisions 4, 5, 6, 7, 8

Raw output: `review-raw/30-DECISION-RULES-unit-2.md`. All three peers.

**Verdicts:** Decision 4 KEEP-WITH-EDITS, Decision 5 KEEP-WITH-EDITS, **Decision 6 CUT**, Decision 7
KEEP-WITH-EDITS (and strengthen substantially), Decision 8 KEEP-WITH-EDITS.

#### CRITICAL

**D2-C1. The isolation gate proves the system was disconnected, not that it still worked. TWO-PEER CONVERGENCE.**
Decision 7 (lines 200-223) passes on: zero unexpected outbound traffic (209), the canonical swarm ran **to
completion** while disconnected (210), and an evidence bundle exists (211).

**It never checks that the disconnected output was correct.** Gemini: *"What if agent tools silently fail or fall back
to useless defaults when the internet is unreachable?"* DeepSeek names it: **silent functional degradation**, where
the agent reports nominal success while tool failures are masked by cached defaults, skipped retrieval steps, or
empty-but-valid-shaped results the surrounding code accepts.

**So a disconnected run can complete cleanly, produce worthless output, and pass the gate that tells a client the
system works in isolation.**

**Fix, proposed independently by both:** an **artifact parity check against a connected baseline run.** The
disconnected output must be functionally equivalent, not merely complete.

**DeepSeek's catch on that fix, which Gemini did not raise:** the baseline itself can be contaminated if it was
produced with the same degraded fallbacks, and semantic thresholds can be loose enough to admit worthless results.
**The connected baseline must be captured and hashed BEFORE the isolation test, with its own correctness established
separately**, or the parity check compares two degraded runs and proves nothing.

**D2-C2. Decision 7 does not distinguish a configuration claim from an evidence claim.**
Codex: as written it can read as *"the firewall says egress is blocked"* rather than *"packet capture shows nothing
crossed the wire."* The `<= 5 incidental packets` allowance (209-212) is meaningless unless the capture proves each
packet's destination, protocol, and expected path. **Require pcap/flow artifacts, capture interface names, the time
window, a hash of the evidence bundle, and explicit accounting for every allowed packet.**

This is the third time this distinction has come up (gate-6 and the evidence spec were the others). **It is the single
most repeated defect in the corpus.**

#### HIGH

**D2-H1. THE GAP: no rule anywhere guards against budget bleed.**
Gemini's answer to which of the three new failure modes is unguarded, and it is the most useful finding in this unit:

> "Decision 6 used to be the hard stop because it required writing a massive check. Under rent-first you could burn
> $150K across a year of 'promising' iterative GCP runs and nothing in this document would stop you."

**The old CapEx trigger was doing double duty**: it gated a purchase, and by requiring a large visible cheque it also
functioned as an involuntary stop-and-think. Removing it removes the brake without replacing it. **The rewrite needs a
cumulative OpEx burn limit that forces a pivot or shutdown if crossed without revenue.**

**D2-H2. Decision 4 actively enables lingering.**
Line 136's *"No further Gate 0 runs required unless..."* grants permission to declare the cheap test done and delay
the expensive one indefinitely. **It guards against wasting money on broken code and provides no forcing function to
advance.** Directly instantiates the failure-to-advance mode identified in unit 1. *(Gemini)*

**D2-H3. Decision 8 has no throughput floor.**
Lines 224-240 gate production readiness on soak, concurrency, and fault behavior but never on tokens per second.
**Add `>= 15 tok/s/stream under 4-way batched load`,** the standing production floor. A system can pass every current
condition in Decision 8 and still be too slow for the verification gates to keep up. *(Codex)*

#### DELETION, WITH PURPOSE RECORDED

**D2-D1. Decision 6 (Pantheon Rack tier, lines 176-199): CUT the trigger, keep the question in a new form.**
- **What it was for:** deciding whether to buy an $80K-$500K enterprise GPU rack.
- **Does the problem persist?** The purchase does not. **Capacity planning does**, and so does the commitment
  decision, in a different currency.
- **Replacement (Gemini):** an **Instance Reservation / Committed Use Discount trigger.** The question is no longer
  "do we buy metal" but *"at what sustained utilization do we lock into a one-year contract instead of paying
  on-demand."* Same shape, same discipline, reversible currency.
- Codex adds: retain the validation prerequisites embedded in it; drop only the purchase trigger.

#### KEEP-WITH-EDITS DETAIL

**D2-E1. Decisions 5 and 8 hang off gate numbers that no longer mean anything.**
Both reference gates demoted to pricing-sweep rows. **Rehang them on named validation artifacts:** Decision 5 on a
"core thesis validation run" (worktree creation, merge cleanliness, generated-code validity), Decision 8 on a
"release-candidate production readiness bundle" (soak, concurrency, fault injection, throughput floor). *(Codex)*

**D2-E2. Both also change character under the rebuild.** *(Gemini)* Decision 5 becomes a **unit economics baseline**
rather than a binary thesis check, since its time limits now inform pricing. Decision 8 becomes an **OpEx stress
test**: "no memory leaks" is no longer only about stability, it is about whether long-running tasks force
over-provisioning on rented nodes.

**D2-E3. Decision 4 works locally with only wording changes.** NATS, containers, mock vLLM, and dispatch are
substrate-neutral. Only the "GPU dollars" / "GPU gate" framing (138, 144) assumes cloud. *(Codex)*

#### MEDIUM

**D2-M1. Unresolved dependencies:** gate bundle paths that may be stale (152, 180, 204, 228); "full 4-task canonical
swarm" with no canonical fixture defined (210); "Gate 0 bundle metrics" with no concrete local artifact names (132);
"evidence bundle lands via PGA" with no schema or hash checklist (211). **The canonical swarm is the same missing
fixture the preflight review found.** *(Codex)*

**D2-M2. Decision 5's thresholds are internally coherent:** `>= 80%` validity validates, `< 50%` falsifies, and
50-79% falls to the inconclusive branch. Recorded because it is one of the few places the corpus handles ambiguity
correctly. *(Codex)*

---

### `30-DECISION-RULES.md` unit 1 (lines 1-127): framing, Decisions 1-3

Raw output: `review-raw/30-DECISION-RULES-unit-1.md`. All three peers.

**Headline, and all three reached it independently: the machinery is the asset, the subject matter is the liability.**
Keep the structure, replace the content. This is the same shape as the hypothesis finding in the evidence spec.

#### KEEP (with the reason, per the standing rule)

**D1-K1. The framing at lines 8-28 is sound and must survive intact.**
It separates pre-commitment, mechanical application, an ambiguity fallback, and amendment-instead-of-reinterpretation
(10-12), and the per-rule template cleanly separates Trigger, Evidence source, Rule, Fallback, Amendment log (18-24).

Gemini: *"This is not ceremony. You built this framework to stop yourself from lying to yourself about evidence."*

DeepSeek's enumeration of what carries into any other decision domain: a threshold committed **before** evidence,
mechanical application with no judgement at decision time, an **explicit inconclusive branch**, and an **open
amendment log** where you revise the rule rather than the interpretation.

**D1-K2. The amendment log is the piece most likely to be dropped, and it is the accountability mechanism.**
DeepSeek: *"People keep the threshold and the mechanical trigger, but when the rule produces an uncomfortable outcome
they silently reinterpret the threshold instead of openly amending it. The accountability mechanism is the first thing
to go."* Gemini agrees on why it works: it cannot physically prevent goalpost-moving, but **the friction of having to
write down a bad excuse in a dated log is what breaks the motivated-reasoning cycle.**

**Do not drop the amendment log in the rewrite.** Both peers flagged it unprompted.

**D1-K3. The explicit inconclusive-evidence branch (lines 55-59, 89-91) is good design.**
It forces one rerun, then defaults to the more reversible path. Keep the pattern.

#### CRITICAL

**D1-C1. Line 39 sets `>= 5 tok/s per stream` under 4-way batched load. Standing policy requires 15.**
Third instance of the same defect (see E2-C1 and the preflight review). **A number three times too low, in a
pre-committed rule, in the document that exists to be applied mechanically.** *(Codex)*

**D1-C2. Decision 3's rule threshold is weaker than its own trigger.**
The trigger fires at utilization `> 80% sustained for 4+ weeks` (line 104), but the rule then only requires `> 70% for
30+ days` (line 112). **A rule that is easier to satisfy than the condition that invokes it is not a gate.** *(Codex)*

#### THE QUESTION THAT MATTERS MOST

**D1-Q1. What temptation do these rules guard against now?**
They were written to stop the author rationalizing a $15K purchase he wanted to make. That temptation is gone.
Gemini names the replacement, and it is correct:

> "Without a $15K upfront price tag to give you pause, it is dangerously easy to rationalize leaving a heavy GCP
> instance running overnight so I don't have to wait 5 minutes tomorrow, or to stay in Track B forever tweaking rented
> setups because the hourly cost feels negligible."

**The two new failure modes are zombie infrastructure and failure to advance.** Cheap and reversible decisions do not
trigger the deliberation that expensive irreversible ones do, so the bleed is continuous and nothing ever forces a
stop. **The rewritten rules must guard against those, not against purchases.**

#### MAPPING: Decisions 1-3 are dead, but not for the reason I expected

Gemini: the mapping onto rent-first **fails**, and forcing it would be worse than dropping it. *"Applying 4-week
friction logs (42) and multi-week bottleneck tracking (71) to a $3/hour highly-reversible rental decision is
bureaucratic theater."* The ceremony was proportionate to a $15K irreversible commitment. It is absurd for a decision
you can reverse by pressing stop.

**Codex disagrees in part and is also right:** the underlying *measurements* remain worth taking (throughput,
contention, LoRA completion without OOM, utilization time series, scheduling conflicts). **So: keep the measurements,
drop the ceremony around them.** The measurements belong in the Track B sizing sweep, not in a purchase gate.

#### REPLACEMENT RULES PROPOSED (Gemini, to be refined in the rewrite)

- **Track A exit.** Trigger: local loop configured. Threshold: canonical 8-task swarm >= 6/8 offline. Rule: stop
  tinkering with local infrastructure, advance to Track B. *(Guards against failure to advance.)*
- **Track B baseline lock.** Trigger: sweep data collected. Threshold: cheapest tier sustaining the production floor.
  Rule: that tier becomes the default; renting larger requires a new explicit rule. *(Guards against budget bleed.)*
- **Track C pilot conversion.** Trigger: pilot reaches 30 days or a spend cap. Threshold: client executes a paid
  contract. Rule: convert or terminate. **No perpetual free trials.** *(Guards against zombie infrastructure.)*

**Note the threshold in Track B needs correcting before adoption:** Gemini wrote `>= 50 tok/s for 72B`, borrowing
Decision 2's single-stream number. The standing floor is **15 tok/s/stream at 4-way batch**, which is the condition
policy actually cares about.

#### SALVAGE LIST (extract before removing the rules)

Codex quoted the thresholds worth preserving: contention factors `< 2.0` (40) and `< 1.5x` (80); `32B LoRA <= 3 hrs
without OOM` (41) and `<= 2 hrs` (81); `>= 5 events/week where the 48GB tier specifically mattered` (42); GCP spend
`>$1000/mo for 2 consecutive months on consistent workload` (69); `72B single-stream >= 50 tok/s` (79); `canonical
8-task agent swarm >= 6/8 pass` (82); utilization `> 80% sustained for 4+ weeks` (104) and `> 70% for 30+ days` (112).

Line 39's `>= 5 tok/s per stream` is preserved **only** as a record that it was superseded by 15, so nobody
reintroduces it.

#### MEDIUM

**D1-M1. Line 119 is mislabelled `Fallback`** where the others say `Fallback (INCONCLUSIVE evidence)`, and it handles
negative evidence rather than ambiguous evidence. Two different branches with one name. *(Codex)*

---

### `20-EVIDENCE-BUNDLE-SPEC.md` unit 4 (lines 391-end): storage, retention, versioning, claims

Raw output: `review-raw/20-EVIDENCE-SPEC-unit-4.md`. All three peers. **Completes the review of queue item 2.**

#### CRITICAL

**E4-C1. "Retain forever" and "provably destroy client data" cannot both be true. THREE-PEER CONVERGENCE, and it is
structural.**
Lines 412-414 state as policy: *"All bundles retained forever"* and *"NEVER delete bundles."* Those bundles will
contain client pilot artifacts. The product claim is sovereignty with provable destruction on request.

DeepSeek: *"Policy cannot resolve this. It IS the contradiction. Treating history as a moat is just a rationale, not a
reconciliation."* Gemini: *"You cannot promise clients privacy while hoarding their adjacent pilot data in
perpetuity."* Codex adds the compliance angle: it also conflicts with legal hold release, privacy deletion,
contractual retention limits, and regulated data minimization.

**Minimum structural fix, and it resolves the two-bundle question from unit 1:** split by **data ownership**, not by
audience. Evidence bundles contain only **non-client** data (metadata, hashes, compute logs, anonymized metrics) and
may be immutable and retained. **Client pilot artifacts live in a separately encrypted, deletion-capable store** with
a TTL and a destruction certificate, so erasure never touches the evidence archive.

**Note this upgrades E1-S1.** Unit 1 proposed the split on presentation grounds (do not hand a client your
`Mike's notes`). It is actually a requirement: the sovereignty product cannot ship without it.

**What "never delete" was FOR** (standing rule): overcompensation to justify the moat narrative and to prove compute
spend was not wasted, per line 414's own words, "represent paid compute." **Replacement:** TTL plus a provable
destruction protocol for client data; genuine retention for non-client evidence only.

#### HIGH

**E4-H1. The storage math is wrong by more than 13x.**
Line 406 concludes *"100 runs/year = ~5GB of bundles = $1.20/year. Effectively free forever."* Codex totalled the
table's own per-gate sizes at **~672 MB per full Gate 0-7 run**, so 100 runs is **~67 GB/year and about $16/year**.
The $1.20 figure only works for ~5 GB. **The document contradicts its own table two lines later.**

**E4-H2. The size table contradicts design goal 7 and cannot accommodate the isolation evidence.**
Goal 7 caps a bundle at 100 MB; line 402 claims 500 MB for gate 7. And the 10-20 MB early-gate figures are not
credible for any bundle carrying raw packet captures, which unit 1 established are load-bearing for the isolation
claim. **The size budget and the evidence requirement are in direct conflict and neither is specified.** *(Codex)*

**E4-H3. Four of the seven "What this spec enables" claims are false as written.**
- Semantic search (436): FALSE, no Pythia ingestion is deployed.
- Structured queries (437): FALSE, no Supabase tables, ingestion, or consumers exist.
- Cost accountability (440): FALSE unless the bundle captures real billing data per run.
- Knowledge moat (442): marketing, not a delivered property.
- Reproducibility (439): OVERSTATED. *"See exactly what config produced what result" is auditability, not
  reproducibility.* That distinction is worth keeping in the rewrite.
- Decision audit trail (441): OVERSTATED, and its example is *"why did we buy the RTX Pro 6000?"*, a purchase that was
  cancelled.
- Human-readable archive (438): PARTIAL, the files are readable but the Obsidian integration is not delivered.
*(Codex, with Gemini concurring on 441 and 442)*

**E4-H4. Nearline transition is underspecified.** Line 413 promotes bundles to nearline after 90 days but ignores
nearline's 30-day minimum storage duration and retrieval fees, and does not state how a lifecycle transition
interacts with the retention policy or bucket lock that real immutability requires. *(Codex)*

#### MEDIUM

**E4-M1. `harness/migrations/` (line 425) does not exist.** Another present-tense reference to an unbuilt path.
*(Codex)*

**E4-M2. "Downstream consumers must handle both schemas" (424) is unenforceable** with zero consumers deployed. There
is no compatibility contract yet. *(Codex)*

**E4-M3. "Effectively free forever" (406) omits real charges:** Class A/B operations, retrieval charges outside
Standard, egress, lifecycle and rewrite operations, early-deletion charges, and versioned-object storage if
versioning is used for immutability. *(Codex)*

**E4-M4. "Guard them like production data" (444) is decorative.** With no signing, no immutability enforcement, and
local disk as the destination, the sentence carries no engineering constraint. Gemini: *"You cannot guard a mutable
local JSON file with adjectives."*

#### STRATEGIC

**E4-S1. The hype register runs through the tail and must go.** "Effectively free forever" (406), "NEVER delete"
(414), "your test history IS the moat" (442). Required tone for a document a client security team reads is clinical
and legally unambiguous: compliance boundaries, retention limits, data handling. Not a manifesto. **Same finding as
T-S2 in the preflight review, so it is a corpus-wide edit, not a local one.** *(Gemini)*

**E4-S2. What the rewritten tail should contain.** Replace "Storage economics" with local disk quota and rotation
policy (Track A writes locally). Replace "Retention policy" with the provable-destruction mechanism for client data
plus honest retention for non-client evidence. State the artifact schism explicitly. *(Gemini)*

---

### `20-EVIDENCE-BUNDLE-SPEC.md` unit 3 (lines 322-390): lifecycle, downstream consumers

Raw output: `review-raw/20-EVIDENCE-SPEC-unit-3.md`. All three peers.

#### THE ROOT-CAUSE RULE FOR THE WHOLE CORPUS (adopt in every rewrite)

DeepSeek, asked why unbuilt consumers get written in the present tense with stated latencies:

> "It lets the author present an aspirational design as operational fact, making the specification sound authoritative
> and complete **without confronting the uncomfortable truth that nothing is built.** The one editorial rule that would
> have prevented it: **never use the present tense for behavior that is not implemented and verified; use 'will' or
> 'should' for intended behavior, or mark it explicitly as 'planned / not implemented.'**"

**Every major finding in this review reduces to this.** Six spend layers of which three were prose. A Cloud Function
pasted into a runbook. A Dockerfile copying a directory that never existed. Golden images with no build manifest. A
harness entrypoint pointing at a missing module. And now six automations of which zero are deployed. **In every case
the tense did the lying**, and the specificity is what makes it persuasive: "within 60 sec" reads as evidence of
implementation.

**Rule: present tense is reserved for what has been executed and verified. Everything else is `will`, `should`, or an
explicit `NOT BUILT` marker.**

#### CRITICAL

**E3-C1. Of six claimed automations, ZERO are deployed. Codex audited each against the repo.**
Line 352 says "six automations trigger."
- **Supabase extraction (356-361): DOES NOT EXIST.** No function source, no schema for `pantheon_runs` /
  `run_hypotheses` / `run_metrics` / `run_costs`, no deploy config.
- **Pythia embedding (363-367): PARTIAL.** `.pythia/` index state exists and `lcs_investigate` is referenced in
  skills, but there is no bundle-ingestion pipeline that watches storage, embeds, tags, or inserts.
- **Obsidian sync (369-372): PROSE ONLY.** A template exists and runbooks contain `cp` lines. No automation.
- **Dashboard refresh (374-377): PARTIAL, claimed integration absent.** `dashboard/` exists but reads local daemon
  routes. No Grafana, no Streamlit, no Supabase-backed trend refresh.
- **Hypothesis tracker (379-382): DOES NOT EXIST.** `open-hypotheses.md` and `lessons/candidates.md` not found.
- **Alert on failure (384-387): DOES NOT EXIST.** No subscription, function, or notification config.

The stated latencies are aspirations. **Nothing defines a trigger, queue, retry, SLI, log, or health check, so no
observable contract exists that could prove any of them.**

**E3-C2. The bundle upload is not atomic, and the trigger fires on the wrong object.**
Lines 336-342. Object storage makes uploads visible one object at a time, so a watcher can see `manifest.json` before
`summary.md`, `cost-report.json`, or `metrics/*.json` exist. **The line 356 trigger fires on `manifest.json` write, so
it can fire against an incomplete bundle** and insert partial rows or fail nondeterministically.

**Fix (Codex's minimal correct lifecycle):** stage everything locally, write final artifacts once, upload
non-sentinel files first, **upload a completion sentinel LAST**, and have consumers trigger only on the sentinel after
validating the manifest-declared object set.

#### HIGH

**E3-H1. The bundle can be destroyed by the cleanup that follows it.**
Line 342 uploads, line 345 deletes the VM. A preemption or hard kill between those steps leaves the bundle partial or
absent. `trap` does not reliably survive hard preemption, and `--max-run-duration` deletion can interrupt
finalization. **Upload must be resumable, or finalization must happen off the VM.** *(Codex)*

**E3-H2. The whole pipeline is cloud-triggered for work that is now local.**
Every consumer hangs off a GCS landing event (line 352). With Track A on the Lenovo, that is indirection for its own
sake. Drive it synchronously from the runner's own finalization step. *(Gemini)*

#### DELETION CANDIDATES, WITH PURPOSE RECORDED (standing rule)

**What the pipeline was for, read charitably:** compute cycles normally produce ephemeral shell output rather than
durable intelligence. Writing results simultaneously to relational storage, vector storage, and readable Markdown was
an attempt to make "test history IS the moat" into queryable data instead of a slogan. **That problem is real and the
thesis is sound.** Gemini's verdict on why it failed is the useful part:

> "You build a moat by digging a hole (writing tests). The author instead built an elaborate, event-driven, six-stage
> water filtration plant for a hole they hadn't dug yet. The core thesis, that retained evidence compounds over time,
> remains structurally sound. It simply starved to death waiting for data."

| Consumer | Verdict | What it was for | Replacement |
|---|---|---|---|
| Supabase extraction | DROP | aggregate tracking of cloud runs and costs | the static `manifest.json` in the bundle |
| Pythia embedding | **KEEP, MOVE** | historical runs semantically queryable | local post-run script, not a Cloud Function |
| Obsidian sync | **KEEP, MOVE** | readable reports into a knowledge base | a local copy step |
| Dashboard refresh | DROP | cost-per-insight charts for cloud spend | per-run `summary.md` |
| Hypothesis tracker | **KEEP** | forces human synthesis of raw data | stays a mandatory manual step |
| Alert on failure | DROP | paging when an unattended remote VM failed | non-zero exit code, since execution is local |

**E3-D1. The manual step at 379-382 is correct design and only mislabelled.**
Listing a human review under a heading that claims six automations is careless writing, but the step itself is right.
**Automation cannot synthesize a strategic lesson.** The pipeline should dump formatted evidence and halt, forcing a
human to review before the master belief state changes. Fix the label, keep the gate. *(Gemini, and I agree)*

---

### `20-EVIDENCE-BUNDLE-SPEC.md` unit 2 (lines 48-321): required file schemas

Raw output: `review-raw/20-EVIDENCE-SPEC-unit-2.md`. All three peers.

#### CRITICAL

**E2-C1. The canonical metrics example teaches a PASS that fails standing policy, in two different ways.**
Lines 263-303. The example records `tokens_per_second_per_stream_median: 12.4` at `concurrency: 1` against
`targets.tokens_per_second_per_stream_min: 10`, and marks it `"verdict": "PASS"`.

Standing policy (rescued from the archived `HARDWARE_DECISION.md`, now in `local-inference-buy-vs-rent.md` section 6)
requires **15 tok/s/stream under 4-way batched load.**

Codex found both defects by reading the file; DeepSeek found both from the numbers alone and added the ranking that
matters:

> "A wrong threshold is a visible numeric error; a wrong load condition invalidates the entire measurement while still
> producing a plausible-looking PASS. People copying the template will repeat the same structurally meaningless test
> setup without noticing."

**The concurrency mismatch is the more dangerous of the two.** 12.4 versus 15 gets caught eventually because it is a
visible number. `concurrency: 1` in a template that everyone copies silently reproduces an experiment that does not
test the thing policy is about, forever, while looking fine. **Fix the threshold, but fix the load condition first.**

**E2-C2. `confidence: 0.85` is fabricated.**
Line 87, inside `decision_rule_outcomes`. Nothing anywhere defines a scoring model, inputs, or calibration. Gemini:
*"pseudo-scientific padding. Submitting baseless confidence scores to a security team will immediately destroy the
credibility of every actual metric in the bundle."* One invented number contaminates every real one beside it.

#### HIGH

**E2-H1. `total_cost_usd` cannot be a required field at finalization.**
Lines 91, 102, 185, 188. The stated attribution method is a label-based query on the GCP billing export, but that
export is written throughout the day rather than in real time, and initial backfill can take up to five days.
**Authoritative cost is necessarily unknown when the bundle is sealed.** The harness was just fixed to refuse to
invent costs, so the schema must accommodate that: emit `cost_status: pending_billing_export` rather than a number.
*(Codex)*

**E2-H2. The schema is saturated with the cancelled purchase.**
`"verdict": "buy 2x 3090 NVLink"` (87), gate-2 hardcoded through the manifest example (57-59), `rtx-3090-proxy` tag
(223), and hypotheses about 70B local inference, concurrent multi-model hosting, and 32B LoRA training (238-245).
A schema specification teaches by example, so it is currently teaching a dead decision as the shape of truth. *(Both)*

**E2-H3. Required-field list is wrong at both ends.**
Line 102. Requires `total_cost_usd` (see E2-H1). Missing for a client-facing artifact: artifact hash/digest, generator
identity and version, git clean/dirty state, schema validation status, evidence completeness status, cost provenance,
per-metric-file checksums, rule and threshold versions, and a signed finalization marker. *(Codex)*

#### MEDIUM

**E2-M1. `nvidia-smi.csv` at a 30-second interval misses most of what matters.** Lines 306-318. Fine for coarse
utilization and thermal drift. Blind to short stalls, bursty saturation, allocation spikes, throttling transients, and
per-process attribution. A reviewer would want per-process usage, driver and CUDA versions, DCGM metrics or finer
sampling, and correlation IDs tying telemetry to specific test windows. *(Codex)*

**E2-M2. Several manifest fields are unpopulatable as specified:** `experimenter` as identity evidence,
`triumvirate_version`/`git_commit` unless captured from the running artifact, `prior_runs_referenced` unless lineage
is enforced, `evidence_bundle_size_mb` unless computed after writing. *(Codex)*

#### DELETION CANDIDATES, EACH WITH ITS PURPOSE RECORDED

Per the standing rule (never delete and hand-wave; it was there for a reason). **In every case here the content is
dead and the structure is the good part.**

**E2-D1. The pre-registered hypothesis structure. KEEP THE STRUCTURE. I dissent from Gemini on this.**
Gemini recommended cutting the "academic hypotheses tested format" (lines 121-130).
- **What it was for:** pre-registering a prediction and a threshold *before* the test runs, so a result cannot be
  rationalized afterward.
- **Does the problem still exist?** Yes, acutely. Post-hoc rationalization is the disease that produced this entire
  review. This is the single most epistemically sound thing in the original corpus.
- **Replacement:** none needed. Replace the *content* (H-2.1 "70B-Q4 local inference is usable") and keep the
  *structure*. Gemini's own suggested replacements are themselves hypotheses, which makes the point.

**E2-D2. `decision_rule_outcomes`. KEEP THE AUDIT TRAIL, CUT THE INVENTED NUMBER.**
- **What it was for:** tracing a verdict back to the evidence that produced it.
- **Does the problem still exist?** Yes. A client asking "why does this say PASS" needs an answer.
- **Replacement:** rule id, rule version, threshold value, measured value, resulting pass/fail. Traceable rather than
  invented. Delete only the `confidence` float.

**E2-D3. `obsidian-note.md`. MOVE IT, DO NOT DELETE IT.**
- **What it was for:** knowledge capture, so runs compound into a searchable vault instead of evaporating.
- **Does the problem still exist?** Yes, and it is the "knowledge moat" the master plan cares about.
- **Replacement:** generate it as an internal sidecar outside the client bundle. `significance: 3` and
  `## Mike's notes` are fine in an internal artifact and disqualifying in a client one. This is the two-bundle split
  from E1-S1, applied.

---

### `20-EVIDENCE-BUNDLE-SPEC.md` unit 1 (lines 1-47): design goals, directory structure

Raw output: `review-raw/20-EVIDENCE-SPEC-unit-1.md`. All three peers.

#### CRITICAL

**E1-C1. The one property a client-facing evidence artifact must have is absent from the design goals entirely.
THREE-PEER CONVERGENCE, reached three different ways.**
Lines 13-19 list seven goals: immutable, self-describing, tool-agnostic, structurally queryable, semantically
queryable, human-readable, cheap. Codex: the missing one is **verifiability / tamper evidence**. Gemini:
**cryptographic verifiability (non-repudiation)**. DeepSeek: **an external append-only anchor outside the writer's
control**.

Gemini's diagnosis of why: *"It is a data engineering brief, not a security brief."* The list is optimized for
Supabase ingestion and developer ergonomics. A skeptical client does not care that the data is cheap to store or
tool-agnostic. They care whether it is true.

**Actionable synthesis:** split the mutable run-state record from the immutable published bundle; make bundle objects
write-once and content-addressed; sign the manifest with a key the tested system never holds; anchor the signature in
an external append-only log. Bucket-level WORM is the second layer, not the first.

**E1-C2. Design goal 1 is contradicted by the spec's own lifecycle section. All three peers caught it.**
Line 13 says "Immutable. Once written, never modified." The lifecycle section (line 322 onward) creates
`manifest.json` at T+0 with `status="running"` and then UPDATES it with verdicts and `ended_at`. That is a mutation.
**The lifecycle should give, not the goal:** keep mutable working state outside the bundle and publish a finalized
bundle once, or make the run-state record and the verdict record separate write-once objects.

#### HIGH

**E1-H1. "Immutable" is declared, never enforced.**
DeepSeek's framing: declaring immutability is a promise, enforcing it is a mechanism, and a reader relying on a
declared-immutable artifact still has to trust whoever controls the storage. Nothing in the artifact reveals a silent
change. The Phase 2 review already established the bucket has only uniform access and public-access-prevention, which
are access controls rather than immutability, with no retention policy, versioning, or bucket lock. *(All three)*

**E1-H2. `raw/` is load-bearing for the isolation claim and nothing requires it to be populated.**
Line 43 describes `raw/` as "raw tcpdump / strace, gate-specific" without mandating any concrete file. **If nothing
populates it, the isolation claim rests on summarized metrics rather than inspectable evidence,** which is exactly the
distinction a client security team will press on. *(Codex)*

**E1-H3. Design goal 7's "< 100MB per run" is incompatible with real packet capture.**
Line 19. A tcpdump across a full gate run easily exceeds that depending on duration, traffic, snap length, and whether
payloads are captured. The spec needs capped captures, metadata-only flow logs, rotation and compression rules, or a
different storage target. **As written, the size goal and the evidence requirement are in direct conflict.** *(Codex)*

**E1-H4. The storage root is hardcoded to GCS.**
Lines 5 and 7: `gs://pantheon-evidence/...` and "No run is considered complete until its bundle lands in GCS." With
Track A local, this couples run completion to cloud synchronization for no reason. Make the root transport-agnostic
and default it to a local filesystem URI. *(Gemini)*

#### STRATEGIC

**E1-S1. This should be two bundles, not one.**
Gemini's split:
- **Client security team:** `manifest.json`, `summary.md`, possibly `artifacts/evaluations/`. Minimal, sanitized,
  signed, verifiable.
- **Internal debugging:** `obsidian-note.md`, hardware metrics, verbose container logs, `raw/` captures.

Handing a client an `obsidian-note.md` (with its "Mike's notes" section) is unprofessional. Handing them raw `strace`
and `tcpdump` output is an active security risk: it leaks infrastructure detail and buries the actual evidence in
noise. **Note the tension with E1-H2:** the client bundle must still contain enough raw material to be checkable, so
the split is between sanitized-and-signed versus everything, not between summary and evidence.

**E1-S2. The independence problem is untouched.** Nothing in lines 1-47 addresses a system generating its own security
evidence. Gemini: *"a system grading its own homework."* Requires an out-of-band observer to assemble the bundle and a
WORM destination the tested system cannot rewrite. *(Gemini, consistent with the earlier method-level finding C-5)*

---

### `10-PREFLIGHT.md` tail: checklist, cost accounting, what comes next (lines 727-779)

Raw output: `review-raw/10-PREFLIGHT-tail.md`. All three peers. **This completes the review of queue item 1.**

#### CRITICAL

**T-C1. The checklist is an attestation layer masquerading as a verification layer. THREE-PEER CONVERGENCE.**
DeepSeek's framing: a human-ticked box converts a question about the world ("does this artifact exist and work?") into
a question about a person's confidence, and confidence is the one thing you cannot afford to trust before irreversible
spending. **A gate whose test cannot fail is not a gate, it is an ornament.**

Note the exact wording at line 750: *"synthetic PubSub test triggered deletion behavior."* **"Deletion behavior" is not
"deleted a VM."** The first asserts a function ran; the second asserts the world changed. A checkbox cannot tell them
apart, and the underlying test could not fail anyway (see P8-C1).

All three peers independently rejected the human-ticked form itself, not merely its contents. The replacement they
converge on: **an automated outcome-based gate that fails closed and writes an evidence bundle.** Each item becomes a
machine check against live state, each check is itself tested against a broken world first to prove it can fail, raw
API output is captured as evidence, and human sign-off is reserved for judgement rather than facts.

**T-C2. Four checklist items reference artifacts that cannot exist.**
Lines 737 (hard-kill deployed AND TESTED), 742 (`pantheon-triumvirate`, `pantheon-test-harness`, `pantheon-vllm-cpu`),
747 (`data/pythia.db`), 748 (`fixtures/`). Counting named artifacts inside those items, at least six. Twenty boxes, and
a fifth of them are unachievable as written. *(Codex)*

#### HIGH

**T-H1. The cost table is not defensible in either row.**
Line 767's "$6-13 one-time" omits the per-gate 500GB pd-ssd at roughly $0.116/hour and undercounts Phase 5. Line 768's
"$15-20/month ongoing" is exceeded by the Phase 5 snapshot alone (~$18) before counting 361GB of models, bucket
storage, per-gate disks, custom images, and Artifact Registry storage of multi-GB images. **Missing from the table
entirely:** Artifact Registry, Cloud Storage across five buckets, custom image storage, per-gate disks, corrected
snapshot pricing, failed and retried builds, logging/monitoring, Pub/Sub and Function costs, and egress. *(Codex)*

**T-H2. The Gemini Ultra credit claim is partly real and wholly unsafe to budget against.**
Line 770: "All within Gemini Ultra GCP credit. Effective cost to Mike: $0." Codex checked current Google sources:
**Google AI Ultra can include monthly Google Cloud credits via the Google Developer Program, so the claim is not
fiction. But it is a bounded benefit, not a blank cheque.** It is falsified by credit exhaustion, an ineligible billing
account or project, expired or unclaimed credits, or services and regions outside the promo terms.

The plan designs its budget around a subscription entitlement rather than the actual billing-account credit balance,
SKU eligibility, and hard caps. Gemini's point lands: with the kill-switch validation also broken, an error here means
uncontrolled personal liability. **Verify the live credit balance before spending, do not infer it from the
subscription.** *(Codex, resolving an assumption flagged three times in this review)*

**T-H3. "What comes next" (line 778) is unsupported in every particular.**
It claims gate-0 costs $0.50, takes ~45 minutes, validates the stack end to end, lands an evidence bundle, and
auto-generates an Obsidian note. Gate-0 cannot run: images missing, harness missing, self-delete unreachable. The only
true statement in the paragraph is that `runbooks/gate-0-plumbing.md` is the next document. *(Codex)*

#### STRATEGIC

**T-S1. The checklist's shape is the tell.** Gemini: exhaustive checklists for unexecuted plans are a psychological
crutch. Twenty granular checkboxes create an illusion of rigor and momentum, letting the author feel work is being
accomplished by documenting a hypothetical state. **This is the same disease the Phase 7 finding named, expressed as a
document artifact rather than as a missing directory.**

**T-S2. Purge the hype register from the corpus.** Line 778's "your first Pantheon run is immortalized" is narrative
payoff, not system state. Gemini's rule for the rewrite: any language that evokes emotion rather than describing
deterministic behavior gets cut. Watch for it elsewhere (the corpus also contains "knowledge moat" and "nuclear
backstop"). *(Gemini)*

**T-S3. What the rewritten tail should contain.** Not a cost table and not a GCP gate-0 preamble. Just: the exact
command that starts the local Track A run on the RTX 4000 Ada, the local path where the evidence bundle is written,
and the specific log output that constitutes success. *(Gemini)*

#### SURVIVING CHECKLIST ITEMS (for the rewrite)

- **Die with the deleted phases:** lines 745 (PD snapshot), 746 (custom VM images), and most of 731-741 in their
  current GCP-first form.
- **Rewrite for local:** 742 (build images locally, drop Artifact Registry), 743-744 (cache weights and checksums to
  local disk), 748 (local fixture paths, after the fixtures are actually authored), 749 (evidence bundle to a local
  directory).
- **Keep but convert to machine checks:** 732 (APIs), 735-736 (budget alert, topic), 738-739 (VPC, firewall), 741
  (Artifact Registry), if and when GCP is actually used.

---

### `10-PREFLIGHT.md` Phase 8 (lines 677-726)

Raw output: `review-raw/10-PREFLIGHT-phase-8.md`. All three peers.

**Phase 8 is the gate that stands between this plan and real money. It validates nothing.**

#### CRITICAL

**P8-C1. The only test of the nuclear spend backstop in the entire corpus proves nothing. THREE-PEER CONVERGENCE.**
Step 8.2 (lines 714-722) publishes a synthetic over-budget message, waits 30 seconds, then asserts
`gcloud compute instances list` is empty. **The document's own comment on line 719 explains the list is empty because
the smoke test already cleaned up** by a completely different mechanism: the VM deleted itself at line 701. So the
assertion is "no VMs remain after the smoke-test cleanup path," not "the kill function killed anything." It passes
identically whether the function works perfectly, is broken, or was never deployed.

DeepSeek names it **The Free Ride**: a vacuous assertion riding on a hidden test dependency. Gemini names it
**tautological testing / validation theater**. Codex reached it by tracing control flow. All three independently.

**The rule (DeepSeek):** couple the assertion to the mechanism under test so it is falsifiable by that mechanism alone.
Verify the precondition, isolate every other actor that could change the observed state, trigger only the mechanism,
then assert. **If you break the kill switch, the test must fail.**

**The correct test (Gemini):** create a persistent VM that does NOT self-delete. Fire an under-threshold alert and
assert it survives. Then fire an over-threshold alert and poll until it is destroyed.

**What a green Phase 8 licenses you to believe (Gemini): that a VM can run a startup script and delete itself. Nothing
about detecting or halting rogue spend.**

**P8-C2. Failure leaves a billable VM running, precisely when things go wrong.**
Line 696's `set -e` means a failed `docker run` (697-699) exits before `gsutil cp` (700) and before self-delete (701).
Same if the container runs but writes no result file. The VM then survives to the 30m cap. So the failure path is
exactly the path that costs money and leaves debris. *(Codex)*

#### HIGH

**P8-H1. The startup-script quoting is broken in three different ways.**
Lines 695-702 mix laptop-side and VM-side interpolation via quote-breaking:
- `$RUN_ID` (line 698) is NOT expanded locally and NOT initialized on the VM, so the container gets `-e RUN_ID=`.
- `'${REGISTRY}'` (line 699) expands on the laptop; unset means the image reference is `/pantheon-test-harness:main`.
- `'$DEFAULT_ZONE'` (line 701) expands on the laptop; unset means `--zone=` with no value.
- `$(hostname)` (line 701) is the only thing that correctly evaluates on the VM.
*(Codex)*

**P8-H2. Self-delete depends on unproven permission and a weak identifier.**
Line 701 uses `$(hostname)` when the real instance name was already known at line 684 and could have been passed in.
And nothing establishes that `pantheon-validator` holds `compute.instances.delete`. *(Codex)*

**P8-H3. `sleep 300` then read is not validation.**
Lines 705-709 wait a fixed interval and then try to `gsutil cat` the result. A failed run surfaces as a confusing
storage error rather than a smoke-test failure. Nothing polls instance status, startup-script exit code, serial
console, or object existence. *(Codex)*

**P8-H4. A non-empty VM list at line 721 is ambiguous and undiagnosable.**
It could mean the startup script failed, the delete permission failed, the kill function failed, or the function never
deployed. The phase provides no discriminator between four very different problems. *(Codex)*

#### CORRECT AS WRITTEN

**P8-OK1.** Image family references at lines 687-688 match Phase 6's `pantheon-orchestrator` family. The
`pantheon-orchestrator-v1` image name is not a mismatch; name and family are separate fields. Recorded so nobody
"fixes" it. *(Codex)*

#### STRATEGIC

**P8-S1. The phase does not survive the rebuild.** With Track A local, the custom images deleted, and the kill function
rewritten or removed, there is no GCP infrastructure left to preflight here. *(Gemini)*

**P8-S2. The same tautology appears inside the thing being tested.** The kill function prints its success string
unconditionally and swallows every exception, so it too "passes" without doing the work. **Validation theater at two
levels: a test that cannot fail, exercising a function that cannot report failure.** *(Gemini, converging with the
Phase 1 finding P1-C4/P1-A1)*

---

### `10-PREFLIGHT.md` Phase 7 (lines 643-676)

Raw output: `review-raw/10-PREFLIGHT-phase-7.md`. All three peers.

**This is the most important section reviewed so far**, not because of its commands but because of what its emptiness
proves about the whole corpus.

#### CRITICAL

**P7-C1. `cd` to a missing directory, then a glob, can upload the repo root to a public-ish bucket.**
Lines 671-672. `cd .../fixtures` fails loudly, but with no `set -e` the next line still runs **from the previous
working directory**, where `*` expands to whatever is there. In this repo that means `gsutil -m cp -r *` could upload
repo-root contents to `gs://pantheon-fixtures/`. A clean failure would be far better than this. *(Codex)*

**P7-C2. `data/pythia.db` does not exist.** Line 650. Codex verified. Step 7.1 cannot run at all. *(Codex)*

#### HIGH

**P7-H1. "1-2 hours, $0" (line 643) prices the upload and ignores the authoring.**
The phase's real content is a curated LoRA training corpus, 12 canonical agent tasks across three languages, scoring
rubrics per task type, and a 50KLOC embedding corpus. None exist. Gemini: the estimate "treats the hardest part of
evaluation as a zero-cost assumption." Codex independently called it a substantial authoring project, not a few JSON
files. *(Both)*

**P7-H2. "Server" is undefined.** Lines 645, 648. Nothing in the corpus defines this capital-S machine. Codex checked:
other docs define `zeus`, `athena`, `vulcan`, `orch`, and Homebox hardware, but not this. An operator cannot know where
to run step 7.1. *(Codex)*

#### MEDIUM

**P7-M1. SQLite `.backup` is the right primitive** (line 650) and should be kept, but the step sets no `busy_timeout`,
checks no exit status, and verifies the resulting DB not at all. *(Codex)*

**P7-M2. `tar czf` around a single `.db`** (line 651) is packaging, not compression. Use `gzip`, or add a checksum if
integrity is the actual goal. *(Codex)*

#### STRATEGIC

**P7-S1. Fixtures belong in git, not in a bucket.**
Canonical test inputs are text. Putting them in `gs://pantheon-fixtures/` creates opaque detached state and breaks
reproducibility, and with Track A local it is pure indirection. `gs://pantheon-fixtures` was already on the Phase 2 cut
list. *(Gemini)*

**P7-S2. The LoRA corpus dies with the cancelled purchase.** The agent tasks, rubrics, and embedding corpus survive
conceptually for local Track A and the pricing sweep, but every one of them still has to be written. *(Gemini)*

**P7-S3. The Pythia export to GCS is another dead-strategy artifact.** Bouncing a local SQLite DB into the cloud only
to pull it down elsewhere serves nothing once Track A is local. *(Gemini)*

#### THE CENTRAL FINDING (two-peer convergence, and it reframes the whole review)

Gemini: *"The total absence of these fixtures proves the plan was an architectural hallucination. The gates were never
going to run because the inputs to run them did not exist."*

DeepSeek, asked only about sequencing and with no knowledge of Gemini's answer: *"The plan was built as a
forward-looking dependency graph rather than a backward-verified chain: later gates were allowed to assume fixtures
that had never been checked into existence, and since no gate was ever run, the missing inputs stayed invisible."*

**The rule both derive, adopt it in the rewrite:** before committing any gate, its canonical fixtures must already
exist and pass a validation check, so downstream work can never depend on unbuilt inputs. Define, author, and version
the inputs in git *before* designing any execution infrastructure.

This explains the entire corpus. Six advertised spend layers with three unimplemented, a Cloud Function that exists
only as pasted prose, a Dockerfile that copies a directory that never existed, golden images with no build manifest,
and a test harness entrypoint pointing at a module nobody wrote. Every one is the same failure: **the document
described a forward dependency graph that nothing ever walked backwards to verify.**

---

### `10-PREFLIGHT.md` Phase 6 (lines 545-642)

Raw output: `review-raw/10-PREFLIGHT-phase-6.md`. All three peers.

**VERDICT: DELETE THIS PHASE TOO.** Both file-reading peers again reached that independently. Gemini's summary is
exact: the "baking" here consists of installing Docker and running `docker pull`, so the portable equivalent is to not
use custom VM images at all and just pull containers at runtime.

#### CRITICAL

**P6-C1. The image family does not exist any more.**
Lines 606-607 use `common-cu126` from `deeplearning-platform-release`. Current DLVM families are
`common-cu129-ubuntu-2404-nvidia-580` and `common-cu129-ubuntu-2204-nvidia-580`, and even CUDA 12.8 was deprecated on
2026-04-13. The naming scheme itself changed to encode CUDA, Ubuntu, and driver version. The command fails outright.
*(Codex, against Google docs updated 2026-08-11)*

**P6-C2. `${REGISTRY}` is unset on both baker VMs, so every pull fails.**
Lines 573-579 and 619-621. `REGISTRY` was exported in Phase 3 in a different shell on a different machine.
`docker pull ${REGISTRY}/${img}` expands to `docker pull /pantheon-triumvirate:main`, and a Docker reference cannot
begin with `/`. This fails as an invalid reference before any network call. *(Codex)*

**P6-C3. Spot plus DELETE destroys the artifact being built.**
Lines 605, 611-612. If the baker is preempted mid-bake, `--instance-termination-action=DELETE` deletes the VM and its
partially prepared boot disk, so there is nothing left to image. Spot suits idempotent restartable work; this is an
interactive SSH bake. Use a standard VM, or at minimum `STOP`. *(Codex)*

#### HIGH

**P6-H1. The "L4 + A100 + RTX Pro 6000 compatible" claim at line 597 is likely false.**
Line 604 bakes on an L4, and line 613's `install-nvidia-driver=True` installs drivers for the *attached* hardware.
Snapshotting after that bakes Ada L4 drivers into the image. Booting it on Ampere A100 or Blackwell RTX PRO 6000 risks
driver mismatch, CUDA failure, or silent performance degradation. *(Gemini)*

**P6-H2. `--metadata=install-nvidia-driver=True` contradicts the comment beside it.**
Line 600 says the Deep Learning image has drivers baked; line 613 then asks Google to install them on first boot with a
reboot. Current DLVM families ship driver 580 pre-installed, so the flag should be omitted. *(Codex)*

**P6-H3. No update story, so the images are stale on the next commit.**
Lines 588, 632 hardcode `-v1` names; lines 574-577, 620-621 hardcode `triumvirate:main`. Any push to `main` makes the
baked image stale, and keeping the fast-boot benefit means re-running a multi-hour bake on every code change. Same
lifecycle hole as Phase 5's snapshot. *(Gemini)*

#### MEDIUM

**P6-M1. `newgrp docker` is unreliable in a scripted SSH block.** Lines 568-569; the subsequent `docker pull` at line
578 may not run as expected. Use `sudo docker pull` or reconnect. *(Codex)*

**P6-M2. No guest cleanup before imaging.** Lines 586-592, 629-635. Imaging a booted machine without clearing SSH host
keys and machine identity duplicates them across every VM created from the image. *(Codex)*

**P6-M3. Cost/duration claim "2-3 hours, ~$1-2" (line 545) is not defensible** once the L4 baker, the boot disks, image
storage, and any retries are counted. *(Codex)*

**P6-M4. `nvidia/cuda:12.6.0-base-ubuntu22.04` (line 625) does exist,** but `--gpus all` needs NVIDIA Container Toolkit;
verify `docker info | grep -i nvidia` rather than assuming the DLVM provides it. *(Codex)*

**P6-M5. Imaging from a stopped instance is structurally correct** (lines 586-592). `--force` is not needed. Recorded
so nobody "fixes" a working thing. *(Codex)*

#### STRATEGIC

**P6-S1. Same GCP lock-in as Phase 5, for less benefit.** A GCE custom image cannot travel to RunPod or AWS, and what
it encodes is "Docker is installed and three images are pulled," which is 2-3 minutes of work at runtime. Spending
2-3 hours plus ongoing image storage to save that, before a single gate has run, is premature optimization. *(Gemini)*

**P6-S2. A GCP image proves nothing to a client pilot.** If the pilot must run on the client's own infrastructure, a
GCE-specific artifact is worthless; the isolation story depends on portable orchestration. *(Gemini)*

#### CORPUS-WIDE PATTERN (third sighting)

DeepSeek: **a snapshot records state, not cause.** A golden image loses which package versions apt resolved, which
image digests were pulled, the base image and kernel, and any audit trail permitting rebuild or verification. Two runs
from the same image name can use different content, invisibly, so "reproducible" becomes unfalsifiable rather than
merely unverified. Minimum fix: a version-controlled declarative build manifest (Dockerfile or Packer) with pinned
package versions and image digests, rebuilt from source rather than snapshotted.

**This is the same defect as P3-H1 (unpinned container tags) and P4-H1 (unpinned model revisions).** Three phases,
one root cause: the corpus consistently captures what happened instead of specifying what should happen. Fix it as one
theme in the rewrite, not as three separate patches.

---

### `10-PREFLIGHT.md` Phase 5 (lines 477-544)

Raw output: `review-raw/10-PREFLIGHT-phase-5.md`. All three peers.

**VERDICT CORRECTED 2026-08-26 BY OWNER. The delete verdict below was too broad and partly rests on a bad number.**

The peers priced this against `gsutil cp` from a **same-region GCS bucket** (the document's own 3-5 minute figure at
line 479). That is not the real alternative once the model cache is also cut. **The real alternative is a cold
download from HuggingFace, which for a large model is closer to 30 minutes.** The economics argument ("saves 3-4
minutes worth $0.05") silently substituted the cheap comparison for the expensive one, and I accepted it.

**And for a client pilot or a live demo, a 30-minute cold start is not a cost-optimization question. It is
disqualifying.** Track C explicitly needs stable, responsive infrastructure.

**The correct answer is conditional on where the node runs, not a blanket delete:**

| Node location | Weight source | Cold start | Verdict |
|---|---|---|---|
| Local Lenovo | download once to local disk | zero after the first time | no caching problem exists |
| GCP | **Hyperdisk ML, read-only-many** (Codex's P5-M4) or a same-region GCS cache | seconds to minutes | keep a fast-mount path |
| RunPod / other | HuggingFace direct (GCS would cost egress) | the 30-minute case | needs its own answer |

**Codex named the right replacement and I filed it away wrongly.** P5-M4 records Hyperdisk ML in read-only-many mode
as the current 2026 primitive for shared read-only model mounts (up to 2,500 instances at <=512 GiB, G2 supported),
and I annotated it "relevant only if this phase survives, which it should not." That dismissed the correct answer
because the verdict had already been accepted. **Hyperdisk ML is the replacement for the PD-snapshot pattern, not a
footnote to its deletion.**

**What actually stands from the peer review of this phase:** the safety bugs (`mkfs.ext4` on an unverified device
path, snapshot-from-partial-copy then delete-source), the corrected pricing, the pd-ssd incompatibility with G4, the
GCP-only portability limit, and the missing lifecycle story. Those are all real and must be fixed in whatever
replaces it.

**What does NOT stand:** "delete this and replace it with nothing." The capability is needed. The mechanism should be
Hyperdisk ML on GCP, local disk locally, and a decided answer for third-party providers.

---

*Original verdict, superseded, kept for provenance:*

~~**VERDICT: DELETE THIS PHASE.** Both file-reading peers reached that independently. It is downstream dead weight (it
stages from the `gs://pantheon-models` bucket that Phase 4's review recommends cutting), its economics are inverted,
and the problem it solves does not exist on a persistent local workstation.~~

#### CRITICAL

**P5-C1. `mkfs.ext4` on an unverified device path can destroy the boot disk.**
Lines 494, 503. The disk is attached without `device-name`, so `/dev/disk/by-id/google-persistent-disk-1` is an
attach-order assumption, not a contract. If it resolves to the wrong disk, line 503 formats it. There is no `lsblk`,
no `readlink -f`, no `set -euo pipefail`, and no confirmation. This is the most dangerous single line reviewed so far.
*(Codex)*

**P5-C2. The snapshot can capture a partially populated filesystem, and the source is then deleted.**
Lines 497, 509, 516, 522-523. The stager has `--max-run-duration=4h` with `--instance-termination-action=DELETE`
against a ~361GB copy. If the copy does not finish, the VM is deleted mid-transfer, line 516 snapshots whatever landed,
and lines 522-523 delete both VM and source disk. Nothing verifies byte count, file count, or hashes before the
destroy. *(Codex, converging with DeepSeek's pattern 3)*

#### HIGH

**P5-H1. Cost claim is wrong in both directions.**
Line 477 claims "$15/mo ongoing." Codex prices ~361GB of snapshot at roughly **$18.05/month** (snapshots bill on
compressed incremental bytes, not the 500GB provisioned size, and model weights compress poorly). Worse, line 537
creates a **500GB pd-ssd per gate VM**, roughly **$0.116/hour per running gate**, about $85/month if one is left up.
That per-VM disk cost appears in no budget anywhere. *(Codex)*

**P5-H2. The optimization loses money by a factor of hundreds.**
It spends $15+/month to save 3-4 minutes of `g2-standard-4` startup, which is worth about $0.05 per run.
**Break-even is roughly 300 gate runs per month.** The plan describes a handful of runs total. *(Gemini)*

**P5-H3. pd-ssd breaks on the machine families the plan most wants to test.**
Lines 488, 536-537. G2 still supports Persistent Disk, but G4 cannot use zonal or regional PD at all and requires
Hyperdisk. A4/A4X/A3 Ultra and N4 families are similarly Hyperdisk-only. So the snapshot-to-pd-ssd pattern fails on
exactly the Blackwell hardware the sizing sweep would target. *(Codex, consistent with the same finding in gate-6)*

#### MEDIUM

**P5-M1. `sudo chown -R $USER` is expanded by the caller's shell, not root.** Line 506. Works if the login user is a
normal Linux account, but OS Login can transform the username. Use `"$(id -u):$(id -g)"`. *(Codex)*

**P5-M2. Snapshotting a mounted disk.** Line 515-516 snapshots while the filesystem may still be mounted and dirty.
The clean sequence is `sync`, unmount, detach or stop, then snapshot. *(Codex)*

**P5-M3. `--snapshot-names` is still valid.** Line 516 is not using a removed flag. Recorded so nobody "fixes" it.
*(Codex)*

**P5-M4. A better 2026 primitive exists.** For shared read-only model mounts, Hyperdisk ML in read-only-many mode is
the current recommendation (up to 2,500 instances at <=512 GiB, and G2 supports it). Per-VM disks from snapshot is the
dated pattern. Relevant only if this phase survives, which it should not. *(Codex)*

#### STRATEGIC

**P5-S1. It is a dependent artifact of a dead strategy.** Line 509 copies from `gs://pantheon-models`. Phase 4's review
recommends deleting that bucket because pulling weights to a non-GCP node costs egress while pulling from HuggingFace
is free. Remove the bucket and this phase cannot run at all. *(Gemini)*

**P5-S2. It creates GCP lock-in that biases future provider choice.** A PD snapshot is a GCP-only primitive and cannot
travel to RunPod. Building it quietly penalizes the multi-provider mobility the rent-first policy depends on. *(Gemini)*

**P5-S3. The snapshot is opaque and irreproducible from birth.** `pantheon-models-v1` is hardcoded (line 518) with a
date-stamped description (line 519), no lifecycle, and no upgrade path. Updating one model means manually rebuilding a
v2, hand-editing the reference at line 537, and garbage-collecting v1. Because Phase 4 does not revision-pin
downloads, nobody can say what is inside it. *(Gemini)*

**P5-S4. Track A does not have the problem this solves.** The local box has persistent storage. Download once,
bind-mount into containers. Cold start is not a thing on a workstation that stays on. *(Gemini)*

**P5-S5. A shared snapshot is incompatible with per-client isolation.** Sharing one snapshot across client projects
needs cross-project IAM, which breaks the tenancy boundary; respecting the boundary means rebuilding it inside every
client project. *(Gemini)*

#### REUSABLE PRINCIPLE (DeepSeek)

For any irreversible step a tired human copy-pastes: **treat every destructive step as untrusted until verified, and
make the wrong action impossible rather than unlikely by embedding verification and abort into the command itself.**

1. **Resolve before you wreck.** Stable identifiers plus a hard mismatch check before formatting, e.g.
   `[ "$(readlink -f /dev/disk/by-id/...)" = "/dev/sdb" ] || exit 1`.
2. **Make the shell refuse to continue.** `set -euo pipefail`, plus preconditions: target not mounted, not `/` or
   `/boot`, required confirmation variables set.
3. **Never chain destroy to untested create.** Verify the snapshot independently (size, mountable, checksum, file
   count) before deleting the source, and put the delete behind a separate explicit command rather than
   `snapshot && delete`.

**This principle applies corpus-wide, not just to Phase 5.** Apply it when rewriting every runbook in the queue.

---

### `10-PREFLIGHT.md` Phase 4 (lines 372-476)

Raw output: `review-raw/10-PREFLIGHT-phase-4.md`. All three peers.

#### CRITICAL

**P4-C1. Caching weights in GCS is an anti-pattern under a multi-provider rent-first policy.**
Line 444. Every pull from `gs://pantheon-models` to a non-GCP node (RunPod, Lambda) incurs GCP internet egress at
roughly $0.08-0.12/GB. The 405B model alone would cost **over $20 in egress every time a node boots**. Downloading
directly from HuggingFace to the rented node is free. Gemini's conclusion: cut the `gs://pantheon-models` bucket
entirely. This reinforces the Phase 2 cut list. *(Gemini)*

**P4-C2. `pip install` fails on Debian 12 before a single byte downloads.**
Lines 403-404. PEP 668 externally-managed-environment blocks system-Python installs. The whole phase halts at the first
command. Fix is a venv (`python3-full python3-venv`, then `/opt/hf/bin/pip install -U "huggingface_hub[cli]"`). *(Codex)*

**P4-C3. The 12h cap can permanently destroy partial work.**
Line 391, `--max-run-duration=12h` with `--instance-termination-action=DELETE`, against a claimed 6-10 hour download of
361GB. Anything not yet copied to GCS is lost with the boot disk when the VM is deleted. There is no checkpointing and
no incremental upload: the copy loop (line 442) runs only after all eight downloads finish. *(Codex)*

#### HIGH

**P4-H1. Checksums establish integrity, never provenance. TWO-PEER CONVERGENCE.**
Lines 410-439 download by repo name with no `--revision` pin, and lines 448-453 then hash whatever arrived. DeepSeek's
framing: the manifest records *what arrived*, not *what was supposed to arrive*. A repo can be updated, retagged,
force-pushed, or hijacked between runs and the manifest would faithfully record the substitute. So "we know exactly
what weights are in the box" is false as stated: you know the bytes, not that they are the authoritative release.

Minimum fix both peers named: pin `--revision <full-git-commit-sha>` and store `repo + commit SHA + file SHA256s`
together. Sign the manifest and attach the publisher's attestation for stronger provenance. Same shape as P3-H1.

**P4-H2. The `find` in the checksum block is misparsed and the manifest is partial.**
Lines 448-453. Without parentheses it evaluates as `( -type f AND -name '*.safetensors' ) OR -name '*.bin' OR -name
'*.gguf'`, so `-type f` guards only the first pattern and directories could reach `sha256sum`. It also excludes
tokenizer, config, and model-index files, so it is not a repo integrity manifest. Paths are relative to each model
root, so verification must run from the same directory. Corrected expression is in the raw file. *(Codex)*

**P4-H3. The cost-verification one-liner reports a falsely reassuring number as the bill grows.**
Line 470. `gsutil du -sh | awk '{print $1*0.020}'` coerces the human-readable size, so `350G` becomes `350` and yields
a correct-looking `$7`. But at `1.2T` awk computes `1.2 * 0.020` and prints **`$0.024/month`**. The check gets more
wrong precisely as the cost gets larger. *(Gemini and Codex, converging)*

#### MEDIUM

**P4-M1. `huggingface-cli` is renamed to `hf`.** Lines 405, 410-438. Current form is `hf download` and `hf auth login`.
*(Codex)*

**P4-M2. Interactive login is wrong for an unattended run.** Line 405 requires pasting a token on a disposable VM, and
the 6-10 hour job then depends on an SSH session staying alive. Use `HF_TOKEN` from Secret Manager plus `tmux`,
`systemd-run`, or a startup script. *(Codex)*

**P4-M3. `gsutil` is legacy.** Lines 444, 452, 456, 470 should use `gcloud storage`. *(Codex)*

**P4-M4. Disk margin is thin and the failure mode is misdiagnosed.** Line 386's 500GB against Codex's measured 361.3GB
of actual repo content leaves little room for cache metadata, partial files, and retries. `/tmp` on Debian 12 is on the
root filesystem, not tmpfs. A separate attached disk mounted at `/models` is the safer design. *(Codex)*

**P4-M5. Line 421's comment is wrong.** It says AWQ; `deepseek-ai/DeepSeek-Coder-V2-Lite-Instruct` is the unquantized
variant. *(Codex)*

**P4-M6. No license or acceptable-use logging.** Nothing records model licenses. If a client pilot serves these
weights, terms compliance (for example Meta's MAU limits) is unmanaged. *(Gemini)*

#### STRATEGIC

**P4-S1. Five of eight models cannot run on the 12GB local box and should be cut from this phase.**
Keep TinyLlama (plumbing), BGE-large (embeddings, ~1.5GB), Whisper-large-v3. Cut Qwen2.5-Coder-32B-AWQ (~18GB),
Qwen2.5-72B-AWQ (~40GB), Phi-4 14B and DeepSeek-Coder-V2-Lite-16B (unquantized, 25-30GB), and Llama-3.1-405B
(multi-GPU). Defer until a rented sizing sweep actually needs them. *(Gemini)*

**P4-S2. The 405B download is indefensible right now.** ~200GB for a Gate 5 that was demoted to a pricing-sweep row
that may never run, burning download hours against the 12h cap and paying storage forever. *(Gemini)*

**P4-S3. Model selection is four months stale.** Gemini's take: TinyLlama is workable but dated for a smoke test;
Qwen2.5-Coder and DeepSeek-Coder-V2-Lite are stale for coding; BGE-large-en-v1.5 should be BGE-M3 or Nomic-Embed-Text
v2; and Llama-3.1-405B is a poor reasoning-to-weight choice now. **Verify current model landscape independently before
acting on specific replacement names.** *(Gemini)*

**P4-S4. All eight repo IDs currently resolve and none are gated.** Codex checked HF metadata for all eight. Good news
worth recording: no license-acceptance blocker stands in the way. *(Codex)*

---

### `10-PREFLIGHT.md` Phase 3 (lines 273-371)

Raw output: `review-raw/10-PREFLIGHT-phase-3.md`. All three peers.

**Verdict: Phase 3 does not execute as written.** Step 3.1 is partly valid, step 3.2 is blocked by missing files, and
step 3.3 is entirely fictional.

#### CRITICAL

**P3-C1. `vllm/vllm-openai:v0.6.5-cpu` does not exist upstream.**
Line 288. Codex checked the Docker Hub tag API: `404`. vLLM publishes CPU images under a separate repo
(`vllm/vllm-openai-cpu`), not as a `-cpu` tag, and `vllm/vllm-openai-cpu:v0.6.5` is also `404`. The GPU image
(line 283) and `nats:2.10-alpine` (line 293) both resolve `200`. *(Codex)*

**P3-C2. Step 3.3 has five independent build failures.**
- Line 366: `-f harness/Dockerfile`: file does not exist.
- Line 340: `COPY requirements.txt .` resolves to `gcp-test-plan/requirements.txt`, but line 349 documents it at
  `harness/requirements.txt`. Neither exists.
- Line 343: `COPY harness/ ./harness/` copies a directory that is not a Python package.
- Line 344: `COPY fixtures/ ./fixtures/`: never existed.
- Line 346: `ENTRYPOINT ["python3", "-m", "harness.runner"]`: module does not exist.
*(Codex)*

**P3-C3. Step 3.2 Cloud Build references three missing files and a broken substitution.**
`cloudbuild.yaml` does not exist at the repo root or in `gcp-test-plan/`. `daemon/Dockerfile` does not exist
(line 308). And line 318 uses `${DEFAULT_REGION}` as if it were a Cloud Build substitution; it is a shell variable and
Cloud Build will not expand it. `SHORT_SHA` (lines 310, 315) is not reliably populated for `gcloud builds submit` from
a local source upload, and unavailable substitutions are replaced with empty strings, producing invalid tags. *(Codex)*

**P3-C4. vLLM v0.6.5 is a late-2024 pin being used to test 2026 hardware.**
Lines 283-284. It predates roughly two years of FlashAttention, continuous batching, and FP8/AWQ quantization work, and
lacks optimization (possibly support) for the local Ada Lovelace sm_89 card and for GCP's Blackwell G4. Testing current
silicon on that runtime yields broken builds or throughput that says nothing about the hardware. *(Gemini, with Codex
concurring that the pin is indefensible unless deliberately targeting a historical runtime)*

#### HIGH

**P3-H1. Build-time supply chain defeats the isolation claim. TWO-PEER CONVERGENCE.**
Lines 283, 288, 293 pull mutable public tags with no digest pinning, no signature verification, no SBOM, and no
provenance. DeepSeek's point is the sharp one: the runtime egress test is *structurally incapable* of detecting what is
inside those images. Dormant or time-triggered callbacks, pre-positioned exfiltration code, vulnerable dependencies,
and internal pivot tooling all survive a clean packet capture, because the measurement window never covers build and a
payload can simply wait out the test.

Minimum remedy both peers named: pin by `@sha256:` digest, verify signatures at pull (cosign/Notary), generate and scan
an SBOM, scan for known vulnerabilities, and record build provenance (command, source commit, dependency tree per
layer). The egress test then remains one control among several rather than the whole proof.

**This survives the owner's terminology correction.** Even defining air-gap as "not on the public internet," a dormant
callback baked in at build time is a real hole.

**P3-H2. Cost claim is materially incomplete.**
Header says "~$2-5 in Cloud Build." Cloud Build itself is plausibly cheap on default pools with a free tier, but the
line ignores Artifact Registry storage entirely, and vLLM GPU images run roughly 8-10 GB compressed. *(Codex)*

#### MEDIUM

**P3-M1. The Python runner duplicates the shell runner that already exists.**
Line 346's `harness.runner` would need `__init__.py`, `runner.py`, and a CLI matching the runbooks' gate/config
semantics. But `harness/runner-wrapper.sh` already owns provision, run, capture, destroy, evidence upload, and gate
config loading. Codex's recommendation: containerize the wrapper rather than invent a parallel Python runner without a
deliberate migration plan. His minimal working Dockerfile is in the raw file. *(Codex)*

**P3-M2. Environment variables assumed to persist across steps.**
Line 280 assumes `DEFAULT_REGION` and `PROJECT_ID` are exported; line 366 depends on `${REGISTRY}` still being set from
step 3.1. A fresh shell produces malformed tags rather than a clear error. *(Codex)*

**P3-M3. `options: logging: CLOUD_LOGGING_ONLY` (line 320) is defensible but unexplained.**
Required only when using a user-specified service account, which must set `logsBucket`, `CLOUD_LOGGING_ONLY`, or
`NONE`. Not inherently required otherwise. *(Codex)*

#### STRATEGIC (whole-phase)

**P3-S1. Registry topology is backwards now that Track A is local.**
Pushing images to GCP Artifact Registry only to pull them back down to the Lenovo adds latency and egress cost for no
benefit. Build and retain locally; defer Artifact Registry until GCP actually runs something. *(Gemini)*

**P3-S2. Images must be loaded from disk, not pulled, for any isolation test.**
If the stack pulls from a registry at runtime, that is a network path, and it is precisely the PGA path the Phase 2
finding describes. Load from a local OCI tarball (`docker load`) or a strictly local registry before the network is
severed, so the test has no pull path at all. *(Gemini)*

**P3-S3. Gemini's cut list.** Cut the vLLM CPU image (lines 287-290; pointless with a local Ada GPU). Cut the Cloud
Build step (lines 300-326; build locally). Cut the containerized test harness (lines 328-369; run via `uv` or a venv).
Defer all `docker push` (lines 285, 290, 295, 367). Only **vLLM GPU** and **NATS** earn their place for Track A, both
pinned by digest.

---

### `10-PREFLIGHT.md` Phase 2 (lines 186-272)

Raw output: `review-raw/10-PREFLIGHT-phase-2.md`. All three peers.

#### CRITICAL

**P2-C1. Private Google Access in the baseline subnet pre-invalidates the air-gap claim. THREE-PEER CONVERGENCE.**
Line 201 enables `--enable-private-ip-google-access`. That gives every instance without an external IP a live route to
Google's public API endpoints. Gate 6 later claims to prove air-gap while explicitly permitting the Google API ranges,
so the "deny-all" rule is really default-deny with an allowlist, and GCS is a writable destination reachable by anyone
with the right access. Codex reached this from firewall mechanics, Gemini from architecture, DeepSeek from claim logic.

DeepSeek's wording of the strongest honest claim, reusable verbatim in the gate-6 rewrite:

> "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to Google API
> ranges via Private Google Access, and during the test window after applying deny-all-egress, fewer than 5 outbound
> packets were observed."

**OWNER CORRECTION (Mike, 2026-08-25), read this before acting on the above.** Air-gap here does not have to mean
literal physical isolation, especially for testing and experimentation. The working definition is **not connected to
the outside world, meaning the public internet**. Private Google Access reaches Google APIs over Google's private
backbone, not the public internet, so **PGA does not automatically fail that definition and does not need to be
ripped out.**

**So the finding is a terminology problem, not an architecture problem.** Both readings are correct in their own
context and the rewrite must hold them apart:

- **For our own testing:** PGA-enabled with default-deny egress genuinely satisfies "not on the public internet." Keep
  it. It is what makes evidence upload possible without a public IP.
- **For a client-facing sovereignty claim:** a security team WILL make the distinction, because data written to GCS
  over PGA is retrievable from the public internet by anyone holding credentials. That is an exfiltration path in the
  strict sense, and calling it "air-gapped" in a proposal is the kind of claim that ends an engagement.

**The fix is precise language, not network surgery.** Do not label the GCP test "air-gap proof." Use DeepSeek's wording
or something close to it, which is accurate under both readings:

> "The workload has no public IP and no general internet egress. Outbound traffic is blocked except to Google API
> ranges via Private Google Access, and during the test window after applying deny-all-egress, fewer than 5 outbound
> packets were observed."

The local box remains the stronger demonstrator for a client who demands literal isolation, because it can be
physically unplugged. That is an option to offer, not a requirement to impose on our own test rig.

**P2-C2. The Phase 1 service account cannot run most of Phase 2.**
Phase 2 never says who executes these commands. If it is `pantheon-validator`, nearly everything fails:
`compute.instanceAdmin.v1` grants no `compute.networks.create`, `compute.subnetworks.create`, or
`compute.firewalls.create` (lines 192-194, 197-201, 204-209, 213-218). `storage.objectAdmin` is object-level and grants
no `storage.buckets.create`, so all five bucket creates fail (lines 229-256). `artifactregistry.reader` is read-only, so
the repo create fails (lines 262-265). `gcloud auth configure-docker` (line 268) configures the local user's Docker
helper, not the runtime SA. *(Codex)*

#### HIGH

**P2-H1. `curl -s ifconfig.me` is a fragile way to build a firewall rule.**
Line 212. Can return empty, HTML error text, or IPv6. Empty expands line 218 to `--source-ranges=/32`. IPv6 with `/32`
is invalid CIDR. CGNAT, VPN, hotspot, or a changing ISP address makes the rule useless or misleading. Better baseline:
no public SSH ingress at all. Use IAP TCP forwarding allowing `35.235.240.0/20` to TCP 22, or OS Login with IAP-only
admin access. *(Codex)*

**P2-H2. The egress comment describes a rule that is never created.**
Lines 220-222 claim egress is "allowed to GCS + Artifact Registry + Google APIs only." No rule exists; VPC default
egress is allow-all, which the comment then admits. Anything built between Phase 2 and Gate 6 has unrestricted egress.
Same disease as the rest of the corpus: the document describes the intended end state in the present tense. *(Codex)*

**P2-H3. The evidence bucket has no immutability, only access control.**
Lines 235-238 set uniform bucket-level access and public access prevention. Neither is immutability. No
`--retention-period`, no bucket lock, no object versioning, no lifecycle policy, no explicit `--soft-delete-duration`
(default 7 days is recoverability, not WORM). The system generating the evidence holds the same IAM rights to overwrite
or delete it, which is worthless to an auditor. Needs a retention policy plus bucket lock, and ideally a separate
project where the test system has append-only rights. *(Codex and Gemini, converging)*

#### MEDIUM

**P2-M1. Generic globally-unique bucket names will collide.**
Lines 229, 235, 241, 247, 253. `pantheon-models`, `pantheon-evidence`, `pantheon-fixtures`, `pantheon-runners`,
`pantheon-pythia-corpus` are all plausible names someone else already took. Creation fails with a conflict. Add a
project or environment suffix. *(Codex)*

**P2-M2. The flat internal firewall is fine solo, dangerous shared.**
Lines 204-209 allow all tcp/udp/icmp across the entire `/20`. Acceptable for a disposable rig, a lateral-movement risk
the moment a Track C client pilot shares the subnet. *(Gemini)*

**P2-M3. MTU 1500 is a deliberate non-default and is unexplained.**
Lines 192-194. GCP's VPC default is 1460, valid range 1300-8896. 1500 is defensible but the doc should say why, since
GKE dataplane behavior can inherit VPC MTU. *(Codex)*

#### STRATEGIC (whole-phase)

**P2-S1. Four of five buckets are dead weight.**
Cut `pantheon-models` (250GB cache for demoted gates), `pantheon-fixtures` (corpora confirmed never to have existed),
and `pantheon-runners` (GCP VM startup scripts, useless if Track A is local). Defer `pantheon-pythia-corpus` unless the
local box pulls from it. Keep `pantheon-evidence`, rebuilt with immutability controls. *(Gemini)*

**P2-S2. The only GCP resources needed right now are Artifact Registry and a hardened evidence bucket.**
With Track A local, the VPC, subnet, firewall rules, and SSH ingress (lines 188-223) serve no immediate purpose.
Artifact Registry serves container images to the local hardware; the evidence bucket receives proofs. Everything else
waits. *(Gemini)*

**P2-S3. Track C cannot share this project.**
It was built as a disposable rig, and Phase 1's kill-switch deletes every VM in the project with no label filter. A
client pilot needs stable uptime, tenant isolation, IAP, and destruction logging. Gemini's position: a dedicated
project per client. At minimum the kill-switch must filter by label before any pilot shares the project. *(Gemini,
reinforcing P1-H2)*

---

### `10-PREFLIGHT.md` Phase 1 (lines 11-185)

Reviewed by Codex (engineering) and Gemini (strategic). DeepSeek (adversarial logic) pending as background task.

#### CRITICAL

**P1-C1. The hard-kill function is Gen1 code deployed as Gen2. It will not work.**
Line 129. The handler signature `hard_kill(event, context)` is the Gen1 background-function shape. Gen2 Pub/Sub
functions use CloudEvents via `@functions_framework.cloud_event`, and the payload sits at
`cloud_event.data["message"]["data"]`, not `event['data']`. The deploy may succeed while invocation never calls the
handler correctly. This is the nuclear backstop for spend control. *(Codex)*

**P1-C2. Gen2 deploy will fail on missing APIs.**
Line 31. `cloudfunctions.googleapis.com` alone is insufficient. Gen2 is backed by Cloud Run and Eventarc, so
`run.googleapis.com` and `eventarc.googleapis.com` must also be enabled. Google's Pub/Sub trigger docs name Artifact
Registry, Cloud Build, Cloud Run Admin, Eventarc, Logging, and Pub/Sub explicitly. *(Codex)*

**P1-C3. The deploy path does not exist.**
Line 159. `cd .../harness/functions/hard-kill` points at a directory that has never existed. The code at lines 113-154
is the only copy and it lives inside this markdown file. Needs `main.py` plus a `requirements.txt` declaring
`google-cloud-compute`. *(Codex, previously confirmed independently)*

**P1-C4. The kill loop hides every failure.**
Lines 142-151. It guesses zone suffixes a/b/c/d per region (not every region has those, and some have others), and
catches every `Exception` with a bare `continue`. That silently swallows missing permissions, disabled APIs, auth
failures, throttling, and delete failures. It can print "Hard-kill completed" having deleted nothing. *(Codex)*

**P1-C5. Deletes are fire-and-forget and cover only instances.**
Line 149. `client.delete()` returns a long-running operation and the code never waits, so the function can return
before any VM is actually gone. It also touches nothing else that bills: disks, snapshots, images, reservations,
static IPs, buckets, Artifact Registry. *(Codex)*

**P1-C6. `NameError` on the final print.**
Line 153. `cost_amount` and `budget_amount` are bound inside `if 'data' in event:` but referenced in the closing
`print` outside it. Any delivery without `data` crashes there. *(Codex)*

#### HIGH

**P1-H1. The budget math is incoherent with the gate costs.**
Lines 104-108. A $100/month budget with the nuclear kill at 50% ($50), against a plan whose most expensive single gate
was estimated at $20-40. One gate consumes up to 80% of the kill threshold. A delayed cleanup or a second concurrent
run trips the wire and destroys the environment mid-work. *(Gemini)*

**P1-H2. The kill function's blast radius is never stated, and it is project-wide.**
Lines 123-127, 146-149. It deletes *every* instance across eleven regions with no label, tag, or name filter. The
rebuilt plan's Track C hosts client pilots whose VMs must not self-destruct. If a pilot runs in this project when the
budget signal fires, it is vaporized. Either Track C needs its own project or the kill must filter by label. *(Gemini)*

**P1-H3. `GPUS_ALL_REGIONS` is missing from the quota ask.**
Lines 50-57. New projects start at zero GPU quota and require BOTH the per-model regional quota AND the global
`GPUs (all regions)` quota. Omitting the global one means VM creation still fails after regional approval is granted.
*(Codex)*

**P1-H4. The quota request will likely be denied on its own text.**
Line 59. It asks for 8x A100 (roughly $40/hr to run) and 192 CPUs while stating a $50/run cap and a $100/month total
budget. A Google reviewer reads that as either a compromised account or someone who does not understand the pricing.
The hardware requested vastly exceeds the stated budget. *(Gemini)*

**P1-H5. The A100 quota ask contradicts the rent-first policy.**
Line 52. GCP charges roughly $5.03/hr on-demand for A100 80GB; RunPod charges $1.19-1.60/hr. Requesting 8x A100 quota
on GCP is the wrong provider for that workload. The GCP ask should be small (1-2x L4 for GCP-specific validation) with
heavy GPU work routed elsewhere. *(Gemini)*

#### MEDIUM

**P1-M1. IAM is labelled "minimum required" and is not.**
Line 72. `roles/compute.instanceAdmin.v1` is broad across instances; `roles/storage.objectAdmin` is project-wide across
all bucket objects; `roles/iam.serviceAccountUser` granted project-wide allows acting as any service account in the
project. Separately, Phase 2 creates firewall rules and `compute.instanceAdmin.v1` does not cover that, so the set is
simultaneously too broad and incomplete. *(Codex)*

**P1-M2. Exporting a JSON service account key is the wrong default in 2026.**
Line 84. Google's IAM guidance treats user-managed keys as risky since the private key is exposed in clear text on
creation. Prefer local ADC with `--impersonate-service-account`, attached VM service accounts, or Workload Identity
Federation. *(Codex)*

**P1-M3. Quota display names are not metric names.**
Line 52. `NVIDIA A100 80GB GPUs`, `CPUs (G2)`, `CPUs (G4)` are plausible console display strings, not stable CLI metric
identifiers. For G2/G4 the GPU quota is the gating one and CPU-family quotas may not be separately requestable under
the current quota model. Needs validation against the exact machine types later phases create. *(Codex)*

**P1-M4. `gcloud functions logs read` is brittle for Gen2.**
Line 181. Works, but should be `--gen2 --region=$DEFAULT_REGION` explicitly rather than relying on config fallback.
*(Codex)*

**P1-M5. Budget notification semantics are misdescribed.**
Line 99. Programmatic budget notifications are sent multiple times per day with current status, not only when an email
threshold fires, and arrive even with no usage. This makes the function's `< 0.5` guard load-bearing rather than
belt-and-braces. Field names `costAmount` and `budgetAmount` are correct. *(Codex)*

**P1-M6. Ordering contradiction.**
Lines 95-96 say the function code is in step 1.6 but "deployed in Phase 4"; lines 156-170 deploy it immediately.
Immediate deploy is correct if the kill path is meant to be live during real spend. A budget can route to a topic before
a function exists, and nothing happens. Do not run spend-bearing phases until topic, budget notification permissions,
function deployment, and a synthetic invocation are all proven. *(Codex)*

#### LOW / CORRECT AS WRITTEN

- `gcloud projects create` syntax is current and valid (line 21). Can fail on taken ID, missing permission, or an org
  requiring `--organization`/`--folder`. *(Codex)*
- `gcloud beta billing projects link` works but `beta` is no longer needed; GA is
  `gcloud billing projects link` (line 23). *(Codex)*
- `gcloud functions deploy --gen2` flag surface shown is broadly valid (line 161). *(Codex)*
- `gcloud pubsub topics publish --message=` is fine for the synthetic payload (line 177), but proves nothing until the
  handler is converted to CloudEvent format. *(Codex)*

#### ADVERSARIAL LOGIC (DeepSeek)

**P1-A1. The kill-switch test proves logging, not killing. Independent confirmation of P1-C4.**
The test greps for `"Hard-kill completed"`, which prints unconditionally at the end. Because every Compute call sits
inside `try/except: continue`, that line runs even if every `list` and `delete` failed or no instances existed.

The minimum test that would actually prove the switch works:
1. Inject known state: create one test VM per zone, or mock `InstancesClient` so `list()` returns fixed fake instances
   across the 11 regions x 4 zones.
2. Send a synthetic event where `costAmount / budgetAmount >= 0.5`.
3. Run `hard_kill`.
4. Assert `client.delete` was called once per instance returned by `list`, with correct project, zone, and instance. On
   real GCP, poll until those instances are gone or the delete operations complete.
5. Send a second event under the threshold and assert zero `delete` calls and no state change.

Two peers reached "it can report success having done nothing" from different directions: Codex by reading the exception
handling, DeepSeek by reasoning about what the assertion tests. Treat that as confirmed, not suspected.

#### STRATEGIC (whole-phase)

**P1-S1. Phase 1 is premature at this size now that Track A moved local.**
Gemini's cut list: delete step 1.3 entirely (no large GPU quota is needed to test local plumbing); defer steps 1.4, 1.5,
1.6 to whichever track first needs cloud execution; keep 1.1 and 1.2 only if Track A needs cloud Artifact Registry or
Storage. Worth weighing against the counter-argument that quota approval is not instant, so filing a SMALL ask early
still has value. *(Gemini)*

**P1-S2. Missing entirely for the rebuilt plan's needs.**
No locked buckets or audit log sinks for client-facing evidence capture. No `iap.googleapis.com` enabled and no
firewall/IAM to support Track C's Identity-Aware Proxy requirement (Track C needs stable uptime and IAP, not SSH). No
proof-of-destruction logging: the kill path deletes VMs but exports nothing verifiable about disk wipes. *(Gemini)*

**P1-S3. The `$100/mo absorbed by Gemini Ultra credit` assumption is four months old and unverified.**
If the credit does not exist or does not apply to Compute Engine, this burns real cash immediately. Verify before
planning around it. *(Gemini, and flagged independently in the rebuilt master plan)*
