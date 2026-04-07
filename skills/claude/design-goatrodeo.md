# Design Goat Rodeo — Industrialized Design Spec Review Machine

**Skill:** `/design-goatrodeo`

**Purpose:** Pressure-test a design spec through multiple rounds of design-specific interrogation, live research, and twin review. Auto-resolve only objective design standards. Surface aesthetic and UX judgment calls to the human. Produce a battle-tested design spec with traceable REQ-IDs.

**Philosophy:** The clankers do the work, the executive makes the calls. Same engine as `/goatrodeo`, different domain knowledge.

**Pipeline Reference:** This skill implements Steps 3, 4.5, and 8 of the Agnostic Web Design Pipeline (v3). See `/Users/mikeboscia/projects/peritia/specs/design-pipeline-spec-v3.md` for the full pipeline context.

---

## Activation

When this skill is invoked:

1. User provides or points to a design spec (brief, IA doc, design system, or full package)
2. Identify which **gate mode** applies based on the document:
   - **Gate 1** (post-brief): Input is a Design Brief. Interrogate strategy, positioning, user model, business objectives.
   - **Gate 1.5** (post-IA): Input is an Information Architecture doc. Interrogate structure, navigation model, user flows, content hierarchy.
   - **Gate 2** (pre-build): Input is the full design package (brief + IA + tokens + interaction specs + component specs). Interrogate everything.
3. Ensure every requirement has a `REQ-###` ID. If not, add them now.
4. Confirm the spec is loaded, print the REQ count and gate mode, and say:
   > **Design Goat Rodeo loaded.** [N] requirements tagged. Gate mode: [1/1.5/2]. Say "go" to start.
5. When user says "go" — execute the machine. Do not stop until decisions are ready.

**Argument:** `/design-goatrodeo [path-to-spec]` — if provided, read that file. If omitted, look for `.design/DESIGN_BRIEF.md` or the most recent design artifact in the project.

**Gate auto-detection:** If the spec contains token definitions → Gate 2. If it contains a sitemap/navigation model but no tokens → Gate 1.5. Otherwise → Gate 1.

---

## Pre-Round: Gather Context

Before Round 1 starts, check existing knowledge infrastructure:

1. **Brand guidelines** — scan for brand guide files in the project (colors, typography, forbidden combinations). These are constraints the interrogator must respect.
2. **Existing design artifacts** — scan `.design/` for any prior design deliverables (briefs, tokens, IA docs). Feed context into every step.
3. **Reference site analysis** — check for reference site research docs. Competitive visual context matters for design review.
4. **Live oracle daemons** — call `mcp__inter-agent__list_daemons`. If one exists for this project, query it for relevant design context.
5. **Content model** — scan for content model specs (CPTs, taxonomies, field groups). Content structure constrains design decisions.

If none exist, proceed without. This step is opportunistic, not blocking.

---

## Per-Round: The State Machine

Print `[ROUND N — STEP M — GATE X]` before each step. No step starts without the previous step's output.

### Step 1: Spec Ready

The design spec exists with REQ-IDs. Either the original (Round 1) or updated with previous round's decisions.

### Step 2: Design Interrogator

Run design-specific interrogation against the spec. The interrogator destroys certainty about design decisions — it does not suggest or build.

**Gate 1 questions (Design Brief):**
- Who is the primary user? What is their emotional state when they arrive? What decision are they trying to make?
- What aesthetic philosophy is named? Is it specific enough to generate tokens from? Or is it a vague descriptor ("modern", "clean")?
- What are the anti-references? What should this explicitly NOT look or feel like?
- Are the brand constraints complete? Missing any colors, forbidden combinations, typography rules?
- What is the content model? How does content structure constrain layout options?
- What are the performance budgets? Do they conflict with the animation ambitions?
- Who is the named decision maker? What happens when design opinions conflict?
- What is explicitly out of scope? Is the boundary clear enough to prevent scope creep?
- How does this site differentiate from competitors visually? What's the one thing someone will remember?
- Are accessibility requirements specified to a WCAG level? A/AA/AAA has massive implications for color choices.

**Gate 1.5 questions (Information Architecture):**
- Is the navigation model appropriate for the content volume? Too many top-level items? Too deep?
- Do the user flows account for all buyer types? Or does the IA assume one persona?
- Are there orphan pages — content that exists but has no clear navigation path?
- Does the URL structure support SEO goals? Are canonical rules defined for dynamic content?
- Is the content hierarchy per page backed by user research, or is it an assumption?
- What happens when content grows? Does the IA accommodate 10x the current volume?
- Are naming conventions consistent? Same concept called different things on different pages?
- Does the sitemap.json exist and match the human-readable IA doc?

**Gate 2 questions (Full Package — all of the above, plus):**
- Token consistency: Are all color tokens semantic (not raw hex)? Do token names communicate purpose?
- Typography: Does the type scale create clear hierarchy? Are line lengths controlled at all breakpoints?
- Spacing: Is the spacing scale consistent? Are there one-off values that break the system?
- Dark mode: If in scope, are dark tokens intentional or just inverted? Are shadows adjusted?
- States: Does every interactive element have all states defined (default, hover, focus, active, disabled, loading, error)?
- Animation: Are motion tokens defined? Do animation durations respect reduced-motion preferences?
- Performance: Do animation specs stay within the 60fps budget? CSS transform/opacity only, or are layout-triggering properties used?
- Responsive: Do components adapt (not just shrink) across breakpoints? Are touch targets ≥44px on mobile?
- Accessibility: Do all color combinations meet WCAG AA contrast (4.5:1 body, 3:1 large text)? Are focus indicators visible?
- Component completeness: Are all components in the manifest speced? Any referenced but not defined?
- Content contract: Are character limits, truncation rules, and empty states defined for every content slot?
- Asset pipeline: Are all referenced icons/images specified with format, resolution, and optimization rules?

Capture output as a structured Q&A list:
```
Q1 (REQ-003): Token --color-accent-primary (#FF9933) on --color-bg-inverse (#001E4B) — what's the contrast ratio? Does it pass AA?
Q2 (REQ-005): Component expert-card references a "credentials" field but the content contract doesn't define max character length.
Q3 (REQ-004, REQ-007): The scrollytelling timeline specifies GSAP ScrollTrigger — what's the no-JS fallback?
...
```

### Step 3: Research

Three live data sources. Zero training data opinions. No WebSearch — ever.

**3a. Gemini quick search** (`mcp__gemini__gemini-search`)
Design-specific research targets:
- WCAG contrast ratio calculations and compliance verification
- SOTA design patterns for the specific component/interaction type
- Animation performance benchmarks (GSAP, CSS, Lottie frame budgets)
- Competitor visual analysis (screenshot + critique of specific referenced sites)
- Typography best practices (line length, scale ratios, responsive type)
- Responsive breakpoint behavior for specific layout patterns
- Accessibility standards for specific interaction patterns (ARIA roles, keyboard nav)
- Design system precedents (how did Carbon, Material, Spectrum solve this?)

Run one search per distinct interrogator question that needs external context.

**3b. Existing research corpus** (project files)
- Check `/research/` directories for prior research that answers interrogator questions
- Check brand guidelines for constraint answers
- Check reference site analysis for competitive context
- Check content model specs for content structure answers

**3c. /last30days** (CONDITIONAL)
- Round 1: run by default for recent design trends, accessibility standard updates, or browser capability changes
- Round 2+: only if interrogator raised questions requiring recent context
- Gate question: "Does the interrogator need to know about recent web platform changes?" No → skip.

For each interrogator question, record which source answered it and what the answer was. Most questions die here — especially contrast ratio math and WCAG compliance checks.

### Step 4: Twin Review

Dispatch to both twins via daemon pattern:

**Package sent to each twin (design-specific prompt):**
- The full design spec with REQ-IDs
- Interrogator Q&A with research answers
- Brand guidelines context
- Content model context (if available)
- Reference site analysis (if available)

**Design-specific twin review prompt:**
> Review this design specification. For each REQ, evaluate:
> 1. **Visual hierarchy:** Does the specified layout/typography/color create clear importance levels?
> 2. **Token consistency:** Are design tokens semantic, complete, and internally consistent?
> 3. **Accessibility compliance:** Do color combinations, touch targets, and interaction patterns meet WCAG AA?
> 4. **Responsive behavior:** Will the specified layouts work at all breakpoints without breaking?
> 5. **Animation feasibility:** Can the specified interactions hit 60fps? Are reduced-motion alternatives defined?
> 6. **Content resilience:** Will the layout survive real content (long names, missing images, empty states)?
> 7. **Brand fidelity:** Does the design respect all brand constraints (including forbidden combinations)?
>
> For each REQ, state AGREE or PROPOSE ALTERNATIVE with rationale.

Twins review independently. They do not see each other's responses.

### Step 5: Auto-Resolve

Compare Claude's assessment + both twin responses. Auto-resolve ONLY objective design standards:

**Auto-resolve (bake in, no user input):**
- Contrast ratio math: if a color combination fails WCAG AA (4.5:1 body text, 3:1 large text), fix it
- Touch target size: if any interactive element is below 44×44px on mobile, fix it
- Missing states: if a component spec lacks required states (hover, focus, disabled), add them
- Token lint failures: if a token doesn't conform to DTCG schema, fix the schema issue
- Semantic HTML: if heading hierarchy is broken, fix it
- Reduced motion: if animations lack `prefers-reduced-motion` handling, add it

**NEVER auto-resolve (always surface to human):**
- Aesthetic direction (color choices, typography pairing, animation style)
- Layout decisions (grid structure, content priority, responsive reorganization)
- Breakpoint values (these are product decisions, not objective standards)
- Brand interpretation (how to apply guidelines is judgment, not math)
- Content hierarchy (what's most important on a page is a business decision)
- Animation philosophy (what should move vs. what shouldn't is creative direction)

**The test: "Would a senior designer consider this a creative decision or an objective standard?"**
- Creative decision → surface it
- Objective standard → auto-resolve
- Unsure → surface it

### Step 6: Frame Decisions

Everything that wasn't auto-resolved becomes the user's decision list.

For each decision:
- Map to REQ-ID(s)
- Frame as 2-3 approaches with visual/UX trade-offs
- Include Claude's recommendation and why
- Note which twin said what
- Reference specific research findings that informed the options

**Design-specific framing guidance:**
- For color decisions: show the hex values, contrast ratios, and which brand rule applies
- For layout decisions: describe the visual result at each breakpoint, not just the CSS
- For animation decisions: describe what the user sees, the performance cost, and the a11y fallback
- For typography decisions: describe the reading experience, not just the font specs

**Janus gate check** (optional, ~1 in 10):
If a disagreement meets ALL THREE conditions:
1. Both twins gave explicit visual/UX reasoning
2. Affects 2+ REQ-IDs
3. Positions are mutually exclusive (can't blend)

Then run `/janus` on it. If emergence found → option C added. If not → standard options stand.

---

## Decision Surfacing

Present the round's results:

```
═══════════════════════════════════════════
  DESIGN GOAT RODEO — ROUND [N] — GATE [X]
═══════════════════════════════════════════

  Auto-resolved: [X] items (objective standards baked in)
  Research answered: [Y] interrogator questions
  A11y fixes applied: [Z] (contrast, touch targets, states)

  Needs your call: [W] items
    1. [Title] (REQ-###, REQ-###)
    2. [Title] (REQ-###)
    3. [Title] (REQ-###, REQ-###, REQ-###)

  Say "go" to walk through them.
═══════════════════════════════════════════
```

When user says "go", feed decisions **one at a time**:

> **Decision [M] of [W]: [Title]**
>
> [Plain english — what the design question is and why it matters for the user experience. 3-5 sentences max.]
>
> **Approaches:**
> A. [Option] — [visual/UX tradeoff + which twin favored this]
> B. [Option] — [visual/UX tradeoff + which twin favored this]
> C. [Option from Janus, if it fired] — [tradeoff]
>
> **Recommendation:** [A/B/C] — [one sentence describing the user experience this creates]
>
> What's your call?

Capture the decision. Move to next. After the last decision in the round:

---

## Round Transitions

**Round 1 → Round 2:** Automatic. Update spec with user's calls. Fire Round 2 immediately. No "ready?" prompt.

**Round 2 → Done (or Round 3):**
After Round 2 decisions are captured, present:

```
═══════════════════════════════════════════
  DESIGN GOAT RODEO — ROUND 2 COMPLETE
═══════════════════════════════════════════

  Spec updated with your calls.

  Run another round? (y/n)
═══════════════════════════════════════════
```

If yes → fire Round 3. If no → proceed to Decision Ledger.

---

## Decision Ledger

Before the spec is declared final, present the full ledger — every decision from all rounds.

**Two sections:**

**YOUR CALLS** — aesthetic and UX decisions the user made:
```
  1. [Title] (REQ-###, REQ-###)
     [What was decided. What the alternative was. Why this
     direction won. What the user experience looks like.
     2-4 sentences, plain english.]
```

**OBJECTIVE STANDARDS** — auto-resolved items:
```
  1. [Title] (REQ-###)
     [What was fixed. What the standard requires. What was
     non-compliant. 1-2 sentences.]
```

User can challenge ANY item — pick a number and Claude explains the full reasoning. User accepts or overrules.

**Only after user says "done" is the design spec final.**

---

## Constraints

- **All inline.** No agents. No background tasks. Everything runs in main chat.
- **No WebSearch.** All web/external searches use `mcp__gemini__gemini-search`.
- **No training data opinions in research.** Step 3 is live sources only. Twins (Step 4) are where design reasoning enters.
- **Print step markers.** `[ROUND N — STEP M — GATE X]` before every step.
- **State machine.** No step starts without the previous step's output in hand. No skipping. No reordering.
- **REQ-ID traceability.** Every interrogator question, research answer, twin comment, and decision maps to REQ-IDs. They carry through the entire design pipeline.
- **Never auto-resolve aesthetics.** The line between objective standard and creative judgment is sacred. When in doubt, surface it.
- **Design domain knowledge.** When interrogating, draw on the 90 design skills from julianoczkowski/designer-skills (7 skills) and Owl-Listener/designer-skills (63 skills, 27 commands). These are the domain knowledge source for design-specific questions.

---

## After the Design Goat Rodeo

Design spec is final. The next step depends on which gate was run:

**After Gate 1 (brief finalized):**
→ Proceed to Step 4 of the design pipeline (IA + Content Architecture)

**After Gate 1.5 (IA finalized):**
→ Proceed to Step 5a of the design pipeline (Structural Tokens)

**After Gate 2 (full package finalized):**
→ Proceed to Step 9 of the design pipeline (Design Freeze)
→ Then invoke `/uncompromising-executor` for the build docs
→ REQ-IDs from the design spec carry into FEAT-IDs

```
Design Goat Rodeo (Gate 1) → battle-tested brief
  ↓
IA + Content Architecture → Information Architecture doc
  ↓
Design Goat Rodeo (Gate 1.5) → battle-tested IA
  ↓
Design System + Interaction + Component Specs
  ↓
Design Goat Rodeo (Gate 2) → battle-tested full package
  ↓
Design Freeze → DDR required for post-freeze changes
  ↓
/uncompromising-executor → canonical build docs
  ↓
superpowers → wave-based execution
  ↓
Visual Verification → Playwright + a11y gates
  ↓
Ship
```
