# Council Decision Record — Output Format

## File Location

```
~/projects/<current-project>/decisions/YYYY-MM-DD_<topic-slug>.md
```

Create the `decisions/` directory if it doesn't exist.

## Standard Format

```markdown
# Council Decision: [Concise Title]

**Date:** YYYY-MM-DD
**Question:** [User's original question, verbatim]
**Council:** Claude (Opus 4.6) · Gemini (Pro) · Codex (GPT-5.2)
**Decision Owner:** Mike Boscia
**Status:** Decided | Needs Validation | Insufficient Data

---

## Context & Forces

[What triggered this council? What are the hard constraints? What's the current state?
2-3 paragraphs max. Include project name, relevant architecture, and timeline pressure.]

## Decision Criteria

[Explicit criteria used to evaluate options, in priority order.]

1. [Criterion — e.g., "Must not require downtime during migration"]
2. [Criterion]
3. [Criterion]

---

## Verdict

[1-3 sentences. What to do. Actionable. No hedging without specifying what the decision depends on.]

---

## Consensus

[Where all three models independently agreed. 3-5 bullets max.]

- [Point]
- [Point]

## Key Tensions

### Tension: [Value A] vs. [Value B] — [Severity]

- **Claude:** [position and reasoning]
- **Gemini:** [position and reasoning]
- **Codex:** [position and reasoning]
- **Resolution:** [trade-off framing, resolution, or "needs validation"]

Severity markers:
- 🔴 **Minority Report** — 2v1 split, dissenting view elevated
- 🟡 **Open Trade-off** — genuine tension, user decides based on priorities
- 🟢 **Resolved** — false dichotomy or clear resolution found

[Repeat for 2-3 most significant tensions.]

## Blind Spots

[What no model addressed. 2-4 items.]

- [Blind spot]

## Confidence Map

| Aspect | Confidence | Signal |
|--------|-----------|--------|
| [Aspect] | High | [e.g., "All three align, evidence-based"] |
| [Aspect] | Medium | [e.g., "Gemini and Claude agree, Codex uncertain"] |
| [Aspect] | Low | [e.g., "Models diverge, needs validation"] |

## Alternatives Considered

| Option | Pros | Cons | Disposition |
|--------|------|------|-------------|
| [Option A] | [pros] | [cons] | **Selected** / Rejected — [why] |
| [Option B] | [pros] | [cons] | Rejected — [why] |
| [Option C] | [pros] | [cons] | Rejected — [why] |

## Accepted Trade-offs

[By choosing the verdict, what pain are we consciously accepting?]

- [Trade-off — e.g., "Higher initial complexity in exchange for operational flexibility"]

## Assumptions

[What must remain true for this decision to hold?]

- [Assumption — e.g., "Supabase row-level security performs adequately at 10K concurrent users"]

## Signals to Revisit

[When should this decision be reconsidered? Checkboxes for tracking.]

- [ ] [Trigger — e.g., "If write volume exceeds 1K/sec"]
- [ ] [Trigger — e.g., "If a third integration needs the same data shape"]

## Next Steps

1. [Most urgent / highest confidence action]
2. [Action that resolves the most uncertainty]
3. [Action informed by key tension]
4. [Optional: longer-term action]

---

## Raw Perspectives

<details>
<summary>Claude (Opus 4.6) — Architecture & Developer Experience</summary>

[Full analysis from Step 3]

</details>

<details>
<summary>Gemini (Pro) — System Health & Scale</summary>

[Full Gemini daemon response]

</details>

<details>
<summary>Codex (GPT-5.2) — Implementation Feasibility</summary>

[Full Codex daemon response]

</details>
```

## Compact Format

For straightforward questions where a full ADR is overkill. Use sparingly — default to standard.

```markdown
# Council Quick Take: [Title]

**Date:** YYYY-MM-DD | **Council:** Claude · Gemini · Codex

**Verdict:** [1-2 sentences]

**Agreement:** [What all three said]
**Disagreement:** [Where they diverged]
**Key Risk:** [Single biggest risk identified]
**Next Step:** [Single most important action]
```

## Chat Display

After writing the decision record to disk, display in chat:

```
Council Decision Record written to: [full path]

**Verdict:** [verdict text]

**Key Tensions:**
- [Tension 1 — one-line summary]
- [Tension 2 — one-line summary]

Full analysis on disk. Follow up with questions — sibling daemons are still active.
```
