# RequirementLinter Analysis

**Repo:** github.com/jonverrier/RequirementLinter
**Source:** INCOSE Guide to Writing Requirements (GtWR) + UK Gov/Miro/Medium user story guides
**Architecture:** LLM-as-judge. Feeds guidelines as prompt context to GPT-5, asks for rule-by-rule evaluation. Two-pass: evaluate, then split compounds.

## Validation Rules Worth Adopting

### Ambiguity Detection (highest value for spec gate)
- **R7 Vague Terms:** Blocklist of ~30 words: "some", "any", "several", "flexible", "sufficient", "appropriate", "efficient", "reasonable", etc.
- **R8 Escape Clauses:** "where possible", "if necessary", "as appropriate", "to the extent practical"
- **R9 Open-Ended Clauses:** "including but not limited to", "etc.", "and so on"
- **R10 Superfluous Infinitives:** "to be designed to", "to be able to", "to be capable of"
- **R26 Absolutes:** "100% reliability", "always", "never", "every", "all"

### Compound Detection
- **R18 Single Thought:** One sentence, one requirement.
- **R19 Combinators:** Flag "and", "or", "unless", "but", "however", "whereas". Signals compound spec.
- Second-pass splitter prompt breaks compounds into atomic statements.

### Measurability
- **R34 Measurable Performance:** Requires specific targets, not "fast" or "responsive".
- **R33 Range of Values:** Quantities need explicit ranges.
- **R35 Temporal Dependencies:** No "eventually", "before", "after" without explicit timing.

### Structure (User Stories)
- **R1-R4:** Must have actor, narrative, goal ("so that..."), and acceptance criteria.
- **R6 INVEST:** Independent, Negotiable, Valuable, Estimable, Small, Testable.

## How to Adapt as Spec Gate

1. **Extract the word blocklists** from R7/R8/R9/R10/R26/R19 -- these are regex-checkable without an LLM.
2. **Combinator scan** (R19) catches compound requirements pre-LLM. Cheap string match.
3. **LLM pass** for deeper checks: measurability, completeness, INVEST scoring. Feed the guidelines markdown as context (they already have it formatted for this).
4. **Splitter pass** for any requirement flagged as compound -- already a separate prompt.
5. **Gate rule:** Block specs containing any R7/R8/R9 term without an explicit override. This alone catches most vague requirements.

The repo's key insight: the guidelines themselves ARE the validation rules. Feed them as LLM context and ask for rule-by-rule citation. No custom ML model needed.
