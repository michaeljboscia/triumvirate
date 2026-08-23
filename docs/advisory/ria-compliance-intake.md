# Small RIA Engagement: Compliance Intake Before Scoping

**Created:** 2026-08-23
**Status:** working template. Driven by a live prospect (small financial planning office, two conversations: one IC, one owner/principal). Not yet used in a real engagement.
**Companion:** `docs/advisory/claude-deployment-options.md` for the platform side.

> **Not legal advice.** This is scoping and architecture. The client's compliance counsel owns the final determination of which regimes apply. Every regulatory citation below needs re-verification against current rule text before it goes in front of a client.

---

## The rule: you cannot scope the AI engagement

"Financial planning office" spans several unrelated regulatory regimes. Until registration status and service mix are known, any scope is a guess. So the engagement is two phases, and the first one is billable:

1. **Compliance assessment.** Short, fixed-fee, produces a written deliverable the client needs regardless of whether they ever adopt AI.
2. **AI adoption**, scoped from phase one's findings.

This is better business than scoping AI work directly. The assessment is fast, it generates the phase two scope from facts instead of assumptions, and if it surfaces a live gap (see Reg S-P below) you found it as their advisor rather than selling into it blind.

---

## Two findings that reframe the work

### 1. The Reg S-P deadline has already passed

The amended Regulation S-P compliance date for **"smaller entities" was June 3, 2026**. "Smaller entity" means an RIA under $1.5B AUM, so essentially any small planning office qualifies.

They are already required to have:

- A written **incident response program** to detect, respond to, and recover from a breach of customer information
- **Customer notification procedures** on a 30 day clock
- **Service provider oversight**
- **Records documenting compliance** with the above

If those do not exist, the firm is not preparing for a deadline. It is out of compliance now. Ask early and without drama, because the answer decides whether phase one is an assessment or a remediation.

**The connection that matters:** service provider oversight is a Reg S-P requirement, and adopting an AI vendor is a service provider engagement. The AI question is not adjacent to their Reg S-P obligation, it is a subset of it. The AI vendor has to be onboarded into that program, so the program's state gates the AI scope.

### 2. Tax preparation is a criminal-liability trap

Many planning firms prepare returns or have a CPA affiliate under the same roof.

If so, transmitting taxpayer return information to an AI vendor is generally a **disclosure under IRC §7216**, carrying criminal penalties, with civil penalties under §6713 alongside. It requires prior written consent that **specifically identifies the recipient**. "Various AI tools" is explicitly insufficient. Those consents almost certainly do not exist.

The FTC Safeguards Rule separately requires a written information security program that names approved AI tools, who may use them, and what data may be entered.

This does not change the seat recommendation. It adds a hard conditional: seats are fine, and *what data may enter them* is a separate question that may require client consents nobody has collected. Recommending seats without asking about tax work is the mistake.

---

## The intake

### Tier 1: Registration status

Sets the base regime. Nothing else can be scoped until this is settled.

| Question | What it determines |
|---|---|
| SEC-registered, state-registered, or both? (AUM is the proxy, roughly $100M) | Advisers Act and Rule 204-2 versus state rules, which vary meaningfully by state |
| Any broker-dealer registration, or registered reps? | FINRA 3110 supervision, 2210 retail communications (AI-drafted client content may need principal pre-approval), SEA 17a-4 WORM retention |
| Insurance licensed? | State insurance regulation on top |

### Tier 2: Services that pull in additional regimes

| Question | What it triggers |
|---|---|
| Tax preparation? | IRC §7216 and §6713, Circular 230, FTC Safeguards Rule, WISP naming approved AI tools |
| CPA firm or CPAs on staff? | AICPA confidentiality standards |
| CFP marks in use? | CFP Board Code and Standards |
| Custody of client assets? | Custody rule, surprise examination |

### Tier 3: Current state

Where you find out whether phase one is remediation.

- Is there a written incident response program, and when was it adopted?
- Is there a WISP, and is it current?
- Who is the designated CCO or qualified individual?
- What is the existing service provider oversight process?
- What is archived today, in what system, and does it capture anything AI-assisted?

### Tier 4: The layer most people skip

- **What do their own client agreements say** about confidentiality and third-party disclosure? Contracts frequently bind tighter than regulation.
- **What does their E&O and cyber policy require or exclude?** Carriers have been adding AI conditions and vendor-management requirements. An insurer's position can constrain the design more than the SEC's, and nobody thinks to check.
- What do the custodian, CRM, and portfolio accounting contracts already permit?

---

## Product recommendation, once the intake clears

**Claude Team.** $20/seat annual, minimum 2 seats, up to 150. No model training on customer content by default, which is the same commitment that holds on Enterprise and the API. Includes SSO, domain capture, admin control over connectors, Claude Code and Cowork. Seat types can be mixed: standard at $20, premium at $100 for 5x usage, so heavy users can be upgraded selectively.

**Not the API, and not the AWS consumption models.** Those are for building software. A planning office has advisors and paraplanners who want to paste in meeting notes and get a summary. That is a seat product. Selling API access sells them a development project they did not ask for.

**Enterprise only if** dually registered or affiliated with a broker-dealer. Then FINRA supervision and 17a-4 retention get materially stricter and the audit logs, Compliance API, and custom retention controls start earning their price. Otherwise it is procurement friction buying capabilities nobody on staff can operate.

---

## Engagement notes

### The real risk is not the vendor

Their privacy risk is almost certainly not Anthropic. It is that staff are already pasting client data into personal free accounts and nobody knows. Buying sanctioned seats **is** the privacy mitigation, because it produces an approved tool with admin visibility and a contractual no-training commitment. "We are worried about privacy so we have not adopted anything" is strictly the worse position.

### Two buyers, two pitches

Where access exists to both an IC and the principal, they are describing different problems that share one answer:

- **The IC's problem:** "I want to use this and I do not know if I am allowed." They want permission and a tool.
- **The principal's problem:** "am I going to end up in a deficiency letter." They want to not be the person explaining an incident to a regulator.

Pitch each on their own terms. The policy engagement is sold to the principal only. The IC is the champion and the requirements source, not the buyer.

### Handling what the IC tells you

If an IC discloses what they are currently doing with client data on personal accounts, that does not go to the principal as an opening line. It burns the champion and can get them disciplined.

Raise it as a category instead: *in firms your size, staff are typically already using these tools on personal accounts, worth finding out before you decide what your policy says.* True independent of anything the IC said, lands the same point, and lets the principal discover it themselves. If they come back having found it, the urgency sells for you.

### One prospect is not validation

Two conversations at one firm is a strong signal about one firm. The twin review already flagged this business model as having zero customer validation and one warm prospect does not retire that. Treat it as a paid design partner: small scope, real money, deliverables that become templates for the next ten firms. Free pilots produce neither revenue nor evidence that anyone will pay.

---

## Open questions for this prospect

1. SEC-registered, state-registered, or dually registered? The one fact that moves the answer off Team.
2. Headcount, and how many would actually get seats?
3. Do they already have an outside IT or compliance vendor? If two people came independently, either nobody owns this or the incumbent is not covering it.
4. Does the principal know the IC talked to us? Changes how to open.
5. Do they prepare tax returns?

---

## Sources to re-verify before client use

- Reg S-P smaller-entity deadline: Davis Wright Tremaine, Carlton Fields, Sidley (May 2026 advisories)
- IRC §7216 and AI disclosure: Tom Talks Taxes, ComplianceHub analysis of Safeguards/§7216/§6713 convergence
- SEC Rule 204-2 recordkeeping: Smarsh, Kitces on AI compliance frameworks
- Claude plan pricing and terms: claude.com/pricing
