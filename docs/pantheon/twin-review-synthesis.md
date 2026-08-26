# Twin Review, April 2026: provenance and unrecovered findings

**Status:** rewritten 2026-08-26 as an archival index. **The original synthesis is in git at
`7e69353:docs/pantheon/twin-review-synthesis.md`.**

---

## What this was, and what happened to it

In April 2026 both peer agents reviewed the Pantheon plan independently and both rated it **2/10**. The synthesis
called the $20K hardware budget a lie, said the plan was building a weapons factory before it had a target, and
listed the missing business validation.

**The verdict was correct. Nothing happened for four months.**

Git bears this out: created `3f809d0` on 2026-04-16, then no substantive follow-up until `b6b49a7` on 2026-08-23,
which finally opened the hardware question again. In between, one unrelated de-identification pass. Meanwhile the
plan it condemned stayed exactly as written and was never executed.

**So this document is not evidence that the review was wrong. It is evidence that a correct review, clearly stated
and filed, changes nothing on its own.** Gemini's framing: *"It proves the system can generate accurate, harsh
feedback, synthesize it into a clear action plan, and then completely ignore it."*

That is worth more than the review's contents, and it is the reason this file is a rewritten index rather than a
deletion.

## The same defects, found twice, four months apart

Codex compared April's findings against the August review record. **Six defect classes appear in both**, meaning they
were identified, documented, and left in place:

| Defect | April | August |
|---|---|---|
| Hardware economics unsupported | "the $20K budget is a lie" | no crossover, no cost model, parts pricing is not a buy/rent case |
| Cloud tests do not prove the target claim | L4/A100 runs do not de-risk the intended hardware | GCP primitives bias provider choice and do not prove local or sovereign readiness |
| Business validation missing | no ICP, funnel, pilot, SLA, churn model | retainer has no cost per client, token budget, margin, or break-even |
| Overclaiming | "no cloud", "zero marginal cost", "eliminates hallucinations" | closing sections repeatedly exceed their evidence |
| Reliability and failure handling weak | no checkpointing, replay, idempotency | partial work destroyed, self-delete unreachable, billable VMs linger |
| Security posture self-attested | no client-code security model | no tamper evidence, isolation self-audited, the evidence upload is itself an exfiltration path |

---

## UNRECOVERED: five things April caught that the August review missed

**This is the operative content of this document.** These were found in April, were not fixed, and were *also* not
re-found tonight. They are not covered anywhere in the rewritten corpus.

### 1. Human-in-the-loop operating protocol

April asked how **one person** supervises, debugs, and intervenes in an autonomous multi-agent system.

August found checklist theater and the misuse of human sign-off for facts a command should settle (`T-C1`), but never
addressed the broader question: **what is the operator's actual control surface?** How do you observe a run in
progress, interrupt it safely, and understand what it did? **Nothing in the rewritten corpus answers this**, and it
is more pressing now that Track A runs on a persistent local box rather than a disposable VM.

### 2. Continuous model evaluation and quality regression detection

April: there is no ongoing evaluation program, so quality regressions would go unnoticed.

August found stale model pins and benchmark provenance problems, but **treated model selection as a point-in-time
decision rather than something needing continuous measurement.** A model swap or a provider-side model update can
silently degrade output, and nothing would catch it. **The rewritten corpus has no standing eval.**

### 3. Source-of-truth under concurrent edits

April named concurrent editing and source-of-truth coordination as unresolved.

August found state contamination between runs and teardown failures, **but not this: when multiple agents work in
parallel, what is authoritative, and how are conflicting edits reconciled?** The archived gate-4 tested whether
parallel worktrees merge cleanly, which is a symptom of this question rather than an answer to it.

### 4. Customer discovery as a gate BEFORE building

April's sharper version of the business finding: **prove paying demand before building, run pilots before capital.**

August covered unit economics and questioned the retainer's shape, but **did not make customer discovery an explicit
gate.** The rebuilt decision rules gate advancement on technical readiness (Rule A) and spend (Rule C), and gate a
pilot's conversion (Rule D), **but nothing gates building on demand existing first.** That is the failure April named
as building a weapons factory before having a target, and it is still ungated.

### 5. Security and compliance for client codebases as a whole

August is strong on network isolation and evidence integrity. **April's concern was broader: handling a client's
codebase securely across its whole lifecycle** — isolation between engagements, audit trail, retention, compliance
posture, and what happens to their source after a pilot ends.

The evidence-bundle rewrite established provable destruction for client data, which is one piece. **The rest is not
covered.**

---

## What August caught that April missed

Recorded for balance. April was a strategic review and could not have found these, because they require reading the
code:

- The runbooks were **not executable**: nonexistent images, missing build files, broken substitutions.
- The **kill switch was fake**: Gen1 code deployed as Gen2, missing APIs, swallowed exceptions, and a test that
  proved logging rather than deletion.
- The **isolation claim was technically invalid**: a permitted egress path, IPv6 unblocked, and a packet threshold
  with room to exfiltrate a private key.
- The **evidence bundle was not audit-grade**: no tamper evidence, no enforced immutability, non-atomic upload.
- **The fixtures never existed**, which meant no gate could ever have run.

---

## The uncomfortable question

Gemini asked what would make tonight's review different from April's, and refused to flatter:

> "April's review ended in a list of markdown bullet points asking the human to do the work. Tonight's will only be
> different if it results in immediate action. If tonight just produces another document summarizing brutal truths,
> it is exactly the same failure as April."

**Partial answer: tonight's review did not stop at findings.** Every document was rewritten, split, or archived, and
committed as it went. The corpus is materially different rather than annotated.

**But that is not sufficient, and claiming it would be the same mistake.** April produced a correct critique that
nobody executed. August has produced a correct *corpus* that nobody has executed. **Both are paper.**

**The difference will be established by running something**, not by the quality of the rewrite. The first real test
is Track A on the local box, and it costs nothing. Until that happens, this remediation is exactly as unproven as the
plan it replaced, and this section should be read as a live warning rather than a closing flourish.

---

## Successors

- `POLICY-rent-first.md` — the standing policy, extracted from the analysis that could not support it
- `gcp-test-plan/REVIEW-PROGRESS.md` — the full August findings record
- `EXTRACTED-from-archive.md` — what survived from documents that were archived
- `gcp-test-plan/SIZING-SWEEP-METHOD.md` — Track B, replacing six demoted gates
