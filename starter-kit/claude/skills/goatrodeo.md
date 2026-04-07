# Goat Rodeo — Industrialized Spec Review Machine

**Skill:** `/goatrodeo`

**Purpose:** Pressure-test a spec through multiple rounds of interrogation, live research, and twin review. Auto-resolve what the AIs agree on. Surface only what needs human judgment. Produce a battle-tested spec with traceable REQ-IDs.

**Philosophy:** The clankers do the work, the executive makes the calls.

---

## Activation

When this skill is invoked:

1. User provides or points to a spec (or one was just written)
2. Ensure every requirement in the spec has a `REQ-###` ID. If not, add them now.
3. Confirm the spec is loaded, print the REQ count, and say:
   > **Goat Rodeo loaded.** [N] requirements tagged. Say "go" to start.
4. When user says "go" — execute the machine. Do not stop until decisions are ready.

**Argument:** `/goatrodeo [path-to-spec]` — if provided, read that file as the spec. If omitted, use the most recently written spec in the current project.

---

## Pre-Round: Gather Context

Before Round 1 starts, check existing knowledge infrastructure:

1. **Pythia** — call `mcp__pythia__pythia_corpus_health` for the project. If indexed, run `mcp__pythia__lcs_investigate` with queries relevant to the spec's domain.
2. **Live oracle daemons** — call `mcp__triumvirate__list_sessions`. If one exists for this project, query it for relevant context.
3. Store context results — they feed into every subsequent step.

If neither exists, proceed without. This step is opportunistic, not blocking.

---

## Per-Round: The State Machine

Print `[ROUND N — STEP M]` before each step. No step starts without the previous step's output.

### Step 1: Spec Ready

The spec exists with REQ-IDs. Either the original (Round 1) or updated with previous round's decisions.

### Step 2: Interrogator

Run `/ruthless-interrogator` against the spec. Produce hard questions mapped to REQ-IDs. The interrogator destroys certainty — it does not build or suggest.

Capture output as a structured Q&A list:
```
Q1 (REQ-003): What happens when the source API is down?
Q2 (REQ-007): Export to what format exactly?
Q3 (REQ-009, REQ-012): Who are the API consumers — internal only or external?
...
```

### Step 3: Research

Three live data sources. Zero training data opinions. No WebSearch — ever.

**3a. Gemini quick search** (`mcp__gemini__gemini-search`)
- All external/web questions go here
- Current docs, APIs, pricing, standards, best practices
- Pre-digested answers, Gemini's API quota, zero Claude token burn
- Run one search per distinct interrogator question that needs external context

**3b. Pythia** (`mcp__pythia__lcs_investigate`)
- All codebase questions go here
- "Do we already have this?" "How does the current implementation work?"
- Only if Pythia index exists (checked in pre-round)

**3c. /last30days** (CONDITIONAL)
- Round 1: run by default for community sentiment on the spec's domain
- Round 2+: only if interrogator raised questions requiring recent external context
- WebSearch calls inside last30days are REPLACED with `mcp__gemini__gemini-search`
- Gate question: "Does the interrogator need to know what happened in the last 30 days?" No → skip.

For each interrogator question, record which source answered it and what the answer was. Most questions die here.

### Step 4: Twin Review

Dispatch to both twins via daemon pattern (`mcp__triumvirate__spawn_session` or `mcp__triumvirate__ask_session` if already alive):

**Package sent to each twin:**
- The full spec with REQ-IDs
- Interrogator Q&A with research answers
- Pythia/oracle context (if gathered)
- Prompt: "Review this spec. For each REQ, state whether you agree with the current direction or propose an alternative. Explain your reasoning."

Twins review independently. They do not see each other's responses.

### Step 5: Auto-Resolve

Compare Claude's assessment + both twin responses. Resolve ONLY what's safe:

**Auto-resolve (bake into spec, no user input):**
- Factual answers confirmed by research (not opinions)
- Unanimous clanker agreement on implementation detail that does NOT change behavior, scope, or architecture

**The test: "Would the user be surprised to learn we decided this without them?"**
- If yes → surface it
- If unsure → surface it

### Step 6: Frame Decisions

Everything that wasn't auto-resolved becomes the user's decision list.

For each decision:
- Map to REQ-ID(s)
- Frame as 2-3 approaches with trade-offs
- Include Claude's recommendation and why
- Note which twin said what

**Janus gate check** (optional, ~1 in 10):
If a disagreement meets ALL THREE conditions:
1. Both twins gave explicit reasoning (not just preferences)
2. Affects 2+ REQ-IDs
3. Positions are mutually exclusive (can't blend)

Then run `/janus` on it. If emergence found → option C added to the approaches. If not → standard options stand. Never mention Janus to the user unless it produced something.

---

## Decision Surfacing

Present the round's results:

```
═══════════════════════════════════════════
  GOAT RODEO — ROUND [N]
═══════════════════════════════════════════

  Auto-resolved: [X] items (baked into spec)
  Research answered: [Y] interrogator questions

  Needs your call: [Z] items
    1. [Title] (REQ-###, REQ-###)
    2. [Title] (REQ-###)
    3. [Title] (REQ-###, REQ-###, REQ-###)

  Say "go" to walk through them.
═══════════════════════════════════════════
```

When user says "go", feed decisions **one at a time**:

> **Decision [M] of [Z]: [Title]**
>
> [Plain english context — what happened, why it matters. 3-5 sentences max.]
>
> **Approaches:**
> A. [Option] — [tradeoff]
> B. [Option] — [tradeoff]
> C. [Option from Janus, if it fired] — [tradeoff]
>
> **Recommendation:** [A/B/C] — [one sentence why]
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
  GOAT RODEO — ROUND 2 COMPLETE
═══════════════════════════════════════════

  Spec updated with your calls.

  Run another round? (y/n)
═══════════════════════════════════════════
```

If yes → fire Round 3. If no → proceed to Decision Ledger.

---

## Decision Ledger

Before the spec is declared final, present the full ledger — every decision from all rounds.

Each item includes:
- What was decided
- What the alternative was
- Why this direction won
- Plain english — enough context to judge cold without follow-up questions

**Two sections:**

**YOUR CALLS** — decisions the user made during walkthroughs:
```
  1. [Title] (REQ-###, REQ-###)
     [What was decided. What the alternative was. Why this
     direction won. 2-4 sentences, plain english.]
```

**CLANKER CONSENSUS** — auto-resolved items:
```
  1. [Title] (REQ-###)
     [What was decided. What the alternative was. Why the
     clankers agreed. 2-4 sentences, plain english.]
```

User can challenge ANY item — pick a number and Claude explains the full reasoning. User accepts or overrules.

**Only after user says "done" is the spec final.**

---

## Constraints

- **All inline.** No agents. No background tasks. Everything runs in main chat.
- **No WebSearch.** All web/external searches use `mcp__gemini__gemini-search`.
- **No training data opinions in research.** Step 3 is live sources only. Twins (Step 4) are where AI reasoning enters.
- **Print step markers.** `[ROUND N — STEP M]` before every step. User should be able to scroll back and see exactly where things happened.
- **State machine.** No step starts without the previous step's output in hand. No skipping. No reordering. This is the rule that was violated twice before this skill existed.
- **REQ-ID traceability.** Every interrogator question, research answer, twin comment, and decision maps to REQ-IDs. They carry into superpowers `writing-plans` after the Goat Rodeo.

---

## After the Goat Rodeo

Spec is final. Invoke `/uncompromising-executor` to produce the canonical documentation suite from the battle-tested spec:

1. `PRD.md` — features with IDs (FEAT-001), acceptance criteria
2. `APP_FLOW.md` — every screen, route, journey, error handling
3. `TECH_STACK.md` — exact frameworks, versions, dependencies, costs
4. `DESIGN_SYSTEM.md` — colors (hex), typography, spacing, radius, shadows, breakpoints
5. `FRONTEND_GUIDELINES.md` — component architecture, naming, file structure, state management
6. `BACKEND_STRUCTURE.md` — schema, auth, API shapes, storage, migrations
7. `IMPLEMENTATION_PLAN.md` — numbered phases/steps, exact files, feature IDs linked to PRD
8. `TEST_PLAN.md` — acceptance test for every REQ-ID (see below)

REQ-IDs from the spec carry into the executor's output. FEAT-IDs in the PRD map back to REQ-IDs.

**This step is NOT optional.** The executor runs before `writing-plans`. The implementation plan builds from the full canonical doc suite, not from the raw spec alone.

### TEST_PLAN.md — Every REQ Gets a Test

The executor produces this alongside the other 7 docs. Every REQ-ID maps to:

| Column | What it contains |
|--------|-----------------|
| REQ-ID | The requirement being tested |
| Acceptance Criteria | Plain english — what "done" looks like |
| Test Type | Unit, integration, E2E, or manual |
| Pass Condition | Specific, verifiable outcome |
| Pre-Implementation Baseline | Current behavior before building |

```
| REQ-ID  | Acceptance Criteria                         | Test Type   | Pass Condition                              |
|---------|---------------------------------------------|-------------|---------------------------------------------|
| REQ-003 | Stale data shown with banner when source down | Integration | Banner visible, data timestamp > 1hr old    |
| REQ-007 | CSV export contains all visible columns     | E2E         | Downloaded file has N columns matching display |
```

**No orphan REQs.** Every REQ has a row. If a REQ can't be tested, that's a spec problem — surface it to the user before proceeding.

### Post-Implementation: REQ Traceability Gate

After `executing-plans` completes, before `finishing-branch`:

Run every test in TEST_PLAN.md. Produce the **REQ Traceability Matrix**:

```
═══════════════════════════════════════════
  REQ TRACEABILITY — [Feature Name]
═══════════════════════════════════════════

  PASS: 14/17 REQs verified
  FAIL: 2 REQs
  SKIP: 1 REQ (manual test — flagged for user)

  ✅ REQ-001 — Postgres schema created
  ✅ REQ-002 — REST endpoints responding
  ❌ REQ-007 — CSV export missing 2 columns
  ❌ REQ-011 — Alert not firing after 3 missed refreshes
  ⏭️  REQ-015 — Requires manual OAuth flow test
  ...
═══════════════════════════════════════════
```

**Gate rule:** `finishing-branch` CANNOT proceed if any REQ is FAIL. SKIP items require user sign-off.

```
Goat Rodeo → battle-tested spec
  ↓
/uncompromising-executor → 8 canonical docs (including TEST_PLAN.md)
  ↓
writing-plans → implementation plan with REQ-IDs + FEAT-IDs
  ↓
executing-plans → build it
  ↓
REQ traceability gate → run acceptance tests, trace to REQ-IDs
  ↓
finishing-branch → only if all REQs pass or user signs off skips
```
