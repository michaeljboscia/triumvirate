# Raw peer output — 10-PREFLIGHT.md tail sections (lines 727-779)

Checklist, cost accounting, and what-comes-next.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**Checklist as audit instrument, lines 731-750:**

| Line | Item | Audit status |
|---:|---|---|
| 731 | GCP project, billing, Gemini Ultra credit | Verifiable by command/console, but "credit active" needs billing-credit evidence |
| 732 | APIs enabled | Verifiable by command |
| 733 | GPU quota requested | Verifiable by quota request/approval record |
| 734 | Service account + minimum IAM | SA verifiable; "minimum" is human judgement unless policy is specified |
| 735 | Budget alert thresholds | Verifiable by command |
| 736 | Pub/Sub topic | Verifiable by command |
| 737 | hard-kill deployed and tested | **Currently misleading/impossible as written.** No deployable source, Gen1-shaped code deployed as Gen2, exceptions swallowed, success printed unconditionally. "Nuclear backstop verified" is unfalsifiable unless deletion of a live target is proven. |
| 738 | VPC/subnet | Verifiable by command |
| 739 | Firewall rules | Verifiable by command; "Mike's IP" requires current-IP check |
| 740 | GCS buckets | Verifiable, but `pantheon-fixtures` implies artifacts that do not exist |
| 741 | Artifact Registry | Verifiable by command |
| 742 | Docker images built/pushed | Verifiable, but **currently impossible** for `pantheon-triumvirate`, `pantheon-test-harness`, and `pantheon-vllm-cpu` (404 base image) |
| 743 | 8 models cached | Verifiable by command/checksum |
| 744 | checksums | Verifiable by command |
| 745 | PD snapshot | Verifiable by command |
| 746 | custom VM images | Verifiable by command |
| 747 | Pythia corpus backup | **References missing source artifact** `data/pythia.db` |
| 748 | fixtures uploaded | **References nonexistent `fixtures/`** |
| 749 | smoke test/evidence bundle | Verifiable by GCS object checks, but only if the test is meaningful |
| 750 | hard-kill synthetic Pub/Sub test | **Currently impossible-to-fail.** "Deletion behavior" is weaker than "deletion"; it can mean log/handler activity, not proof that the function deleted a live resource. |

**Checklist items referencing artifacts that cannot currently exist: 4 items** (lines 737, 742, 747, 748). Counting named artifacts inside those items: **at least 6** (hard-kill source/function, `pantheon-vllm-cpu`, `pantheon-triumvirate`, `pantheon-test-harness`, Pythia DB backup, fixtures).

**Cost table, lines 758-768: not defensible.**

Line 767 "Total one-time $6-13" undercounts Phase 5 and omits per-gate 500GB `pd-ssd` provisioning at roughly `$0.116/hour` each. It also assumes impossible build/run steps complete.

Line 768 "Ongoing storage (monthly) $15-20" is not defensible. Phase 5's snapshot alone is around `$18/month`, before ~361GB of model payload, persistent bucket storage, per-gate disks, custom images, and Artifact Registry storage for multi-GB images. **Missing entirely:** Artifact Registry storage, Cloud Storage for models/evidence/corpus/fixtures/runners, custom image storage, per-gate persistent disks, snapshot storage at corrected pricing, failed/retried Cloud Build and runtime costs, logging/monitoring, Pub/Sub and Function costs, and network egress.

**Line 770 (the Gemini Ultra credit claim).** Current Google sources say Google AI Ultra **can** include monthly Google Cloud credits through Google Developer Program benefits, but this is a **bounded credit benefit, not proof that "effective cost to Mike: $0."** Sources: Google AI plans page, Google Developer Program Benefits FAQ, Gemini API Google AI plans. Falsified by: any spend not covered by an active applied credit, credit exhaustion, ineligible billing account or project, expired or unclaimed credits, or services/regions outside the promo terms. **Risk: the budget is designed around a subscription entitlement rather than the actual billing-account credit balance, SKU eligibility, and hard caps.**

**Line 778.** Given gate-0 cannot run, the paragraph is not currently true. The only true part is that `runbooks/gate-0-plumbing.md` is the named next document. The `$0.50`, the runtime claim, the validation claim, the evidence bundle, the Obsidian note, and "immortalized" are all unsupported.

**Single change:** replace the checklist with a command-backed preflight gate that writes an evidence bundle and **fails closed**. Each checkbox maps to a command/assertion with expected output, including a real destructive hard-kill test against a disposable live resource. No command evidence, no Gate 0.

---

## GEMINI (strategic angle)

**1. Checklist shape (lines 731-750).** Exhaustive checklists for unexecuted plans are a psychological crutch. Twenty granular checkboxes create an illusion of rigor and momentum, letting the author feel work is being accomplished by documenting a hypothetical state. **It substitutes performative bureaucracy for actual engineering execution.**

**2. Survivors and casualties.**
- **Die:** lines 731-741, 745-747, 750. Tied to GCP billing, networking, and the deleted Phases 5 and 6.
- **Rewrite for local:** line 742 (build images locally, drop Artifact Registry), lines 743-744 (cache weights and checksums to local disk), line 748 (local fixture paths), line 749 (smoke-test evidence to a local directory).

**3. Line 770 failure mode.** If the unchecked assumption fails, the user is exposed to raw GCP costs. Because the hard-kill validation (lines 737, 750) is flawed, an error here means uncontrolled personal financial liability. **Resting the entire cost narrative on one unverified line proves the plan treats cost control as a slogan rather than an engineered constraint.**

**4. Line 778 critique.** "Your first Pantheon run is immortalized." This is grandiose roleplay masquerading as technical documentation. It prioritizes dramatic narrative payoff over system state. Any language in the corpus that evokes emotion or hype rather than describing deterministic behavior must be purged.

**5. Rewritten tail.** Delete the cost table (756-770) and the GCP Gate 0 preamble. The tail should contain only: the exact command to initiate the local Track A run on the RTX 4000 Ada, the local path where the evidence bundle is written, and the specific log output that constitutes a successful local run.

---

## DEEPSEEK (adversarial logic angle)

Asked: what is structurally wrong with a human-ticked checklist as the final gate before irreversible spending?

> Structurally, the problem is that a human-ticked checklist is an **attestation layer, not a verification layer**. It
> converts a question about the world (*does this artifact exist and work?*) into a question about a person's
> confidence. Before an irreversible spend, confidence is the one thing you can't afford to trust.
>
> Three specific structural defects:
>
> 1. **It conflates mechanism with outcome.** "Triggered deletion behavior" asserts that a function ran or a message
>    was delivered. "Deleted a VM" asserts that the world actually changed. A tick box can't distinguish the two, and
>    the item's underlying test couldn't fail because it was satisfied by an unrelated mechanism. **A gate whose test
>    cannot fail isn't a gate, it's an ornament. Non-falsifiability is fatal.**
> 2. **It's self-attested, point-in-time, and evidence-free.** Nothing binds a tick to logs, timestamps, identities, or
>    API responses, and drift after the tick (deployment deleted, permissions rotated) makes it stale.
> 3. **It itemizes activity, not risk.** Twenty boxes about mechanisms don't answer the only question that matters:
>    *what billable state exists right now?*
>
> **What replaces it:** an automated, outcome-based gate run as a pipeline, not a ceremony.
>
> - Each item becomes a machine check against live cloud state: `list all VMs matching tag X → assert result is empty`,
>   or `read IAM policy → assert delete permission absent`.
> - Every check is written against the **end-state**, and the check itself is **tested to fail**: run it against a
>   broken world first, prove it fails, then prove it passes on a good one.
> - Evidence (raw API output, timestamps, resource snapshots) is captured and attached to each item; a green pipeline
>   unlocks the spend, and human sign-off is reserved for judgment, not facts.
> - Precede the irreversible portion with a **dry-run or canary spend** so the gate is validated against the real cost
>   path before the point of no return.
>
> One-line version: replace self-attested proxies ("function triggered") with falsifiable outcome checks ("no VM
> remains") executed automatically, because a wrongly-ticked box costs you money you can't get back, but a wrong
> assertion in an automated gate costs you a failed pipeline run.

**Three-peer convergence on the replacement.** Codex: "a command-backed preflight gate that writes an evidence bundle
and fails closed." DeepSeek: "an automated, outcome-based gate run as a pipeline, not a ceremony." Gemini: the
checklist is "performative bureaucracy substituting for engineering execution." All three independently reject the
human-ticked form itself, not merely its contents.
