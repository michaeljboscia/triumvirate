# Goat Rodeo: Industrialized Spec Review Machine

**Skill:** `/goatrodeo`

**Purpose:** Pressure-test a spec through multiple rounds of interrogation, live research, and twin review. Auto-resolve what the AIs agree on. Surface only what needs human judgment. Ship a battle-tested spec with traceable REQ-IDs.

**Philosophy:** The clankers do the work. The executive makes the calls.

**The rule that was missing:** a spec that describes a system without describing how the human reaches it will build the wrong thing, confidently. Phase 0 now enforces user-path tracing before any architecture review.

---

## Activation

When this skill is invoked:

1. User provides or points to a spec (or one was just written)
2. Ensure every requirement in the spec has a `REQ-###` ID. If not, add them now.
3. Confirm the spec is loaded, print the REQ count, and say:
   > **Goat Rodeo loaded.** [N] requirements tagged. Running Phase 0 pre-flight.
4. Execute Phase 0 (User Story Validation). This is NOT optional. It runs before the user says "go."
5. After Phase 0 passes, say:
   > **Phase 0 passed.** [N] user stories validated. [M] blockers found. Say "go" to start architecture rounds.
6. If Phase 0 finds blockers, surface them BEFORE proceeding. User must resolve or waive each one.

**Argument:** `/goatrodeo [path-to-spec]` — if provided, read that file as the spec. If omitted, use the most recently written spec in the current project.

---

## State Machine Enforcement

The goatrodeo is a state machine. Steps run in order. Skipping steps defeats the purpose. This section defines the enforcement model.

### Completion Tracking

Maintain a running gate log throughout the session. After completing each gate, print:

```
[GATE ✅] Phase 0.1 — Requirement Lint
[GATE ✅] Phase 0.2 — JTBD Mapping
...
```

The gate log is cumulative — every gate checkpoint prints ALL prior gates plus the current one. This makes the state visible in the conversation even after compaction.

### Gate IDs

Every mandatory step has a gate ID. These are the checkpoints:

| Gate ID | Step | Blocks |
|---------|------|--------|
| `P0.1` | Requirement Lint | P0.2 |
| `P0.2` | JTBD Mapping | P0.3 |
| `P0.3` | Trace the Human's Path | P0.4 |
| `P0.4` | Constitution Check | P0.5 |
| `P0.5` | Phase 0 Verdict | P0.6 (if applicable), else R1 |
| `P0.6` | Protocol Integration Gate (conditional) | R1 |
| `R{n}.1` | Spec Ready | R{n}.2 |
| `R{n}.2` | Interrogator | R{n}.3 |
| `R{n}.3` | Research | R{n}.4 |
| `R{n}.4` | Twin Review | R{n}.5 |
| `R{n}.5` | Auto-Resolve | R{n}.6 |
| `R{n}.6` | Frame Decisions | DL (after final round) |
| `DL` | Decision Ledger acknowledged by user | P3.1 |
| `P3.1` | Four-Pass Analyze | P3.2 |
| `P3.2` | INVEST Score | P3.3 |
| `P3.3` | Re-Trace Human's Path | P3.4 |
| `P3.4` | Anti-Pattern Check | P3.5 |
| `P3.5` | Phase 3 Verdict | P4.1 |
| `P4.1` | Canonical Docs produced | P4.2 |
| `P4.2` | Trait-to-Task traceability verified | P5.1 |
| `P5.1` | Execution plan with reality tests | P5.2 |
| `P5.2` | Worktree commit SHA gate | P6.1 |
| `P6.1` | Build complete (all tasks) | P6.2 |
| `P6.2` | BUILD_MANIFEST produced | P7.1 |
| `P7.1` | Reality Gate — stub detection | P7.2 |
| `P7.2` | Reality Gate Verdict | P8.1 |
| `P8.1` | Post Rodeo audit | P8.2 |
| `P8.2` | Retrospective written | FINAL |

### Enforcement Rules

1. **No skipping.** Before starting any step, verify its prerequisite gate is in the log. If not, print: `⛔ GATE VIOLATION — cannot run [step] without completing [prerequisite] first.` Then run the missing step.
2. **"Done" requires Phase 3.** When the user says "done" on the Decision Ledger, Phase 3 runs AUTOMATICALLY. Do not ask "want to run Phase 3?" — it is mandatory. The user said "done" on the LEDGER, not on the PROCESS.
3. **Phase 3 does NOT end the goat rodeo.** Phase 3 ends the SPEC REVIEW. Phases 4-8 (build + verify + audit) run next. The goat rodeo owns the full lifecycle: spec to ship.
4. **Final requires all gates.** Before declaring READY TO SHIP, print the full compliance checklist (see below). Every gate must show ✅ or ⏭️ (explicitly waived by user). No gate may show ❌. "Final" means P8.2, not P3.5.
5. **Waiver is explicit.** A gate can only be skipped if the user says "waive [gate ID]" or "skip [gate ID]." Silent skips are violations.
6. **Reality tests are mandatory.** Every task in the implementation plan MUST have a `<reality_test>` that a stub cannot pass. If a task has only `<verify>` (compilation check) and no `<reality_test>`, the plan fails validation. (CRYSTALLIZED — Pythia v2 2026-04-07)
7. **Worktrees require committed state.** Before creating ANY worktree or dispatching ANY parallel agent, verify Wave 0 is committed to git and record the SHA. (CRYSTALLIZED — Pythia v2 2026-04-07)

### Final Compliance Checklist

Print this BEFORE declaring the spec final:

```
═══════════════════════════════════════════
  GOAT RODEO — COMPLIANCE CHECKLIST
═══════════════════════════════════════════
  Phase 0 — User Story Validation
    ✅ P0.1  Requirement Lint
    ✅ P0.2  JTBD Mapping
    ✅ P0.3  Trace the Human's Path
    ✅ P0.4  Constitution Check
    ✅ P0.5  Phase 0 Verdict
    ⏭️ P0.6  Protocol Integration Gate (N/A)

  Round 1
    ✅ R1.1  Spec Ready
    ✅ R1.2  Interrogator
    ✅ R1.3  Research
    ✅ R1.4  Twin Review
    ✅ R1.5  Auto-Resolve
    ✅ R1.6  Frame Decisions

  Round 2
    ✅ R2.1–R2.6  (all gates passed)

  Decision Ledger
    ✅ DL    User acknowledged

  Phase 3 — Post-Review Quality Gate
    ✅ P3.1  Four-Pass Analyze
    ✅ P3.2  INVEST Score
    ✅ P3.3  Re-Trace Human's Path
    ✅ P3.4  Anti-Pattern Check
    ✅ P3.5  Phase 3 Verdict

  Phase 4 — Canonical Docs + Execution Plan
    ✅ P4.1  Canonical Docs (8 docs produced)
    ✅ P4.2  Trait-to-Task Traceability

  Phase 5 — Build Execution
    ✅ P5.1  Execution plan with reality tests
    ✅ P5.2  Worktree commit SHA gate
    ✅ Gate 1  Engine works
    ✅ Gate 2  Graph works
    ✅ Gate 3  Full system works

  Phase 7 — Reality Gate
    ✅ P7.1  Stub scan + reality tests
    ✅ P7.2  Reality Gate Verdict

  Phase 8 — Post Rodeo Audit
    ✅ P8.1  /postrodeo (6 phases)
    ✅ P8.2  RETROSPECTIVE.md written

  ALL GATES PASSED — ready to ship.
═══════════════════════════════════════════
```

If any gate shows ❌, the spec is NOT final. List what's missing and what to do about it.

---

## Phase 0: User Story Validation (NEW — Mandatory Pre-Flight)

This phase runs BEFORE any architecture interrogation. It validates that the spec describes a product humans can actually use, not just a system that internally works.

**Origin:** Triumvirate v2 passed 6 rounds of architecture review, 190 test cases, 13 canonical docs — and shipped a system the user couldn't reach because nobody asked "how does the human call this?" This phase exists to make that impossible.

### Step 0.1: Requirement Lint (Regex, Zero LLM Cost)

Scan every requirement for quality defects. Flag but don't block — accumulate findings.

**Vague term blocklist:** Flag any requirement containing: "some", "any", "several", "flexible", "sufficient", "appropriate", "efficient", "reasonable", "fast", "responsive", "scalable", "robust", "seamless"

**Escape clause blocklist:** Flag: "where possible", "if necessary", "as appropriate", "to the extent practical", "when feasible"

**Open-ended blocklist:** Flag: "including but not limited to", "etc.", "and so on", "such as"

**Compound detector:** Flag requirements containing "and", "or", "unless", "but", "however", "whereas" as likely compound requirements that should be split into atomic statements.

**Measurability check:** Flag any performance or timing claim without a specific number. "Fast startup" → FAIL. "Startup under 5 seconds" → PASS.

Present results:
```
═══════════════════════════════════════════
  PHASE 0 — REQUIREMENT LINT
═══════════════════════════════════════════
  Vague terms: [N] (list REQ-IDs)
  Escape clauses: [N]
  Open-ended: [N]
  Compound: [N] (candidates for splitting)
  Unmeasurable: [N]
═══════════════════════════════════════════
```

### Step 0.2: JTBD Three-Tier Job Mapping

For each major feature area in the spec, define the user's job at three levels:

| Tier | Question | Example |
|------|----------|---------|
| **Functional** | What gets accomplished? | "I get answers from all three agents" |
| **Emotional** | How does the user feel? | "I feel confident they're working, not anxious about silence" |
| **Social** | How is the user perceived? | "I look like I have a team, not like I'm babysitting broken tools" |

**Orphan detection:**
- Every user story MUST trace UP to a parent job. Story without a job = scope creep.
- Every job MUST trace DOWN to at least one story. Job without a story = unimplemented promise.
- Flag orphans in both directions. They are BLOCKERS, not warnings.

### Step 0.3: Trace the Human's Path (CRITICAL GATE)

For EVERY user story or primary flow in the spec:

> "The user just did [the primary action]. Trace every hop from their input to the result they see. Name every process, protocol, and interface boundary."

The trace must include:
1. **What the user types** (literal input — keystroke level)
2. **What they see 1 second later** (acknowledgment)
3. **What they see 5 seconds later** (progress)
4. **What they see 30 seconds later** (result or status update)
5. **What they see on failure** (error + what's being done about it)

**If any hop in the trace is undefined, hand-waved, or references an internal system name the user never sees, the spec FAILS this gate.**

Examples of failing traces:
- "The daemon handles routing" — HOW does the user's message reach the daemon?
- "Agents are managed by the supervisor" — the user doesn't see a supervisor, what do they see?
- "Messages flow through the fabric" — from WHERE to WHERE, through what user-visible interface?

Present each trace:
```
US-1: Ask the twins
  User types: "ask the twins about SQLite WAL"
  1s: Claude says "Sending to both now."
  1s: "→ Gemini: sent ✓"
  1s: "→ Codex: sent ✓"
  8s: "→ Codex: working... (6s)"
  12s: "→ Gemini: working... (10s)"
  15s: "→ Codex: responded ✓"
  20s: "→ Gemini: responded ✓"
  21s: Synthesized results displayed
  FAILURE: "→ Codex: TIMEOUT after 60s ✗ — retrying..."
  
  Hops: Claude session → MCP tool call → Daemon HTTP API → 
        Fabric → Agent connector → CLI subprocess → stdout → 
        Fabric → MCP response → Claude session → user display
  
  ✅ Every hop defined. Interface boundaries named.
```

**GATE:** If ANY primary user story fails the trace, the spec cannot proceed to architecture rounds. Fix the spec first.

### Step 0.4: Constitution Check

If the project has a constitution (core principles that never change), validate every REQ against it.

For the Triumvirate, the constitution is:
1. Claude is the front door. User talks to Claude.
2. Lifecycle is always visible. No silent failures.
3. Plain language in, structured results out. No command ceremony.
4. Failure is loud, immediate, and actionable.

Any REQ that violates a constitutional principle is a BLOCKER.

### Step 0.5: Phase 0 Verdict

```
═══════════════════════════════════════════
  PHASE 0 — USER STORY VALIDATION
═══════════════════════════════════════════
  Lint: [N] warnings, [M] blockers
  JTBD: [N] jobs mapped, [M] orphans
  Path traces: [N] complete, [M] incomplete (BLOCKER)
  Constitution: [N] aligned, [M] violations (BLOCKER)
  
  VERDICT: PASS / BLOCKED ([list blockers])
═══════════════════════════════════════════
```

If BLOCKED: surface blockers to user. User must resolve or explicitly waive each one before architecture rounds begin.

If PASS: proceed to "say go to start architecture rounds."

---

## Pre-Round: Gather Context

Before Round 1 starts, check existing knowledge infrastructure:

1. **Pythia** — call `mcp__pythia__pythia_corpus_health` for the project. If indexed, run `mcp__pythia__lcs_investigate` with queries relevant to the spec's domain.
2. **Live oracle daemons** — call `mcp__inter-agent__list_daemons`. If one exists for this project, query it for relevant context.
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

Dispatch to both twins via daemon pattern (`mcp__inter-agent__spawn_daemon` or `mcp__inter-agent__ask_daemon` if already alive):

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

**When the user says "done":** This means the LEDGER is accepted, NOT that the spec is final. Phase 3 (Post-Review Quality Gate) runs AUTOMATICALLY next. Do not ask "want to run Phase 3?" — it is mandatory. Print `[GATE ✅] DL — Decision Ledger acknowledged` and proceed directly to Phase 3.

---

## Constraints

- **All inline.** No agents. No background tasks. Everything runs in main chat.
- **No WebSearch.** All web/external searches use `mcp__gemini__gemini-search`.
- **No training data opinions in research.** Step 3 is live sources only. Twins (Step 4) are where AI reasoning enters.
- **Print step markers.** `[ROUND N — STEP M]` before every step. User should be able to scroll back and see exactly where things happened.
- **State machine.** No step starts without the previous step's output in hand. No skipping. No reordering. This is the rule that was violated twice before this skill existed.
- **REQ-ID traceability.** Every interrogator question, research answer, twin comment, and decision maps to REQ-IDs. They carry into superpowers `writing-plans` after the Goat Rodeo.

---

## Phase 3: Post-Review Quality Gate (NEW — Runs After Decision Ledger)

After the user says "done" on the Decision Ledger, run this quality gate BEFORE declaring the spec final.

### Step 3.1: Four-Pass Analyze (Adapted from github/spec-kit)

Run four detection passes across the entire spec:

**Pass 1 — Duplication:** Find requirements that say the same thing in different words. Flag pairs with REQ-IDs.

**Pass 2 — Ambiguity:** Re-run the vague term blocklist from Phase 0 against the UPDATED spec (decisions may have introduced new vague language). Also flag requirements where two reasonable people could disagree on what "done" means.

**Pass 3 — Underspecification:** Find verbs without measurable outcomes. "The system handles errors" → underspecified. "The system displays error type, message, and retry option within 2 seconds" → specified.

**Pass 4 — Alignment:** Check every REQ against the JTBD job map from Phase 0.2. Flag any REQ that drifted from its parent job during the architecture rounds. Architecture decisions sometimes mutate product intent — this catches it.

### Step 3.2: INVEST Score

Score every user story against INVEST:
- **I**ndependent — can be delivered without other stories
- **N**egotiable — room for implementation flexibility
- **V**aluable — delivers user-facing value (not just internal plumbing)
- **E**stimable — scope is clear enough to estimate
- **S**mall — fits in one development cycle
- **T**estable — has a concrete pass/fail condition

Flag any story scoring below 4/6. Stories that score 0 on **V**aluable (no user-facing value) are BLOCKERS — they're internal plumbing masquerading as requirements.

### Step 3.3: Re-Trace the Human's Path

Run Step 0.3 AGAIN on the final spec. Architecture decisions may have changed the user-facing flow. The traces must still be complete after all rounds.

### Step 3.4: Anti-Pattern Check

Flag these known failure patterns from real multi-agent projects:

| Anti-Pattern | What To Flag | Source |
|-------------|-------------|--------|
| **Stub trust** | Any feature described without an implementation path | Ruflo: 290/300 MCP tools were stubs |
| **Passive hook** | Any feature that relies on "the agent will notice and act on this" | Ruflo: hooks-as-text doesn't work |
| **Silent failure** | Any error path without user-visible notification | Triumvirate v1: 16% silent failure rate |
| **Invisible infrastructure** | Any system component without a defined user-facing integration point | Triumvirate v2: daemon with no MCP interface |
| **False completion** | Any workflow that reports "done" without verification | CC #32650: 16 failure classes of false completion |

### Step 3.5: Phase 3 Verdict

```
═══════════════════════════════════════════
  PHASE 3 — POST-REVIEW QUALITY GATE
═══════════════════════════════════════════
  Duplication: [N] pairs found
  Ambiguity: [N] requirements flagged
  Underspecification: [N] requirements flagged
  Alignment drift: [N] REQs drifted from jobs
  INVEST: [N] stories below threshold
  Path traces: [N] still complete / [M] broken
  Anti-patterns: [N] detected

  VERDICT: CLEAN / [N] items to resolve
═══════════════════════════════════════════
```

If items to resolve: surface them. User resolves or waives.

After Phase 3 passes (or all items are resolved/waived), print the **Final Compliance Checklist** (defined in State Machine Enforcement above). Every gate must show ✅ or ⏭️. Only then is the spec final.

---

## Phase 4: Canonical Docs + Execution Plan

Spec is final. The goat rodeo does NOT stop here. Phases 4-8 take the spec through build and verification. One command, one skill, spec to ship.

### Step 4.1: Canonical Docs

Invoke `/uncompromising-executor` to produce the canonical documentation suite:

1. `PRD.md` — features with IDs (FEAT-001), acceptance criteria
2. `APP_FLOW.md` — every screen, route, journey, error handling
3. `TECH_STACK.md` — exact frameworks, versions, dependencies, costs
4. `DESIGN_SYSTEM.md` — colors (hex), typography, spacing, radius, shadows, breakpoints
5. `FRONTEND_GUIDELINES.md` — component architecture, naming, file structure, state management
6. `BACKEND_STRUCTURE.md` — schema, auth, API shapes, storage, migrations
7. `IMPLEMENTATION_PLAN.md` — numbered phases/steps, exact files, feature IDs linked to PRD
8. `TEST_PLAN.md` — acceptance test for every REQ-ID (see below)

REQ-IDs from the spec carry into the executor's output. FEAT-IDs in the PRD map back to REQ-IDs.

### Step 4.2: Trait-to-Task Traceability (CRYSTALLIZED — Pythia v2 2026-04-07)

**Origin:** Pythia v2 defined a `CodeGraph` trait in Wave 0 but nobody was assigned to implement `SqliteCodeGraph`. The trait existed without an owner.

**The check:** For EVERY trait/interface defined in BACKEND_STRUCTURE.md Wave 0:
1. Find the task in IMPLEMENTATION_PLAN.md that implements it
2. If no task exists → **BLOCKER.** Add the task before proceeding.
3. If the task exists but is in a worktree → verify the worktree's task list explicitly names the implementation, not just "build against the trait"

Print:
```
Trait-to-Task Traceability:
  ✅ EmbeddingBackend → M-003 (LlamaCppEmbedder)
  ✅ Reranker → M-009 (OnnxReranker)
  ✅ SearchEngine → M-010 (HybridSearchEngine)
  ❌ CodeGraph → NO TASK ASSIGNED ← BLOCKER
  ✅ KnowledgeStore → ORC-003 (SqliteKnowledgeStore)
  ✅ OracleProvider → ORC-001/002 (GeminiCli/ClaudeCli)
```

### TEST_PLAN.md — Every REQ Gets TWO Tests

The executor produces TEST_PLAN.md alongside the other 7 docs. Every REQ-ID maps to:

| Column | What it contains |
|--------|-----------------|
| REQ-ID | The requirement being tested |
| Acceptance Criteria | Plain english — what "done" looks like |
| Test Type | Unit, integration, E2E, or manual |
| Pass Condition | Specific, verifiable outcome |
| **Reality Test** | **A test that a STUB CANNOT PASS (see below)** |
| Pre-Implementation Baseline | Current behavior before building |

### Reality Tests (CRYSTALLIZED — Pythia v2 2026-04-07)

**Origin:** Pythia v2 Codex sessions built LlamaCppEmbedder as a byte-hasher producing fake 1536d vectors. Tests passed because they only checked `vec.len() == 1536`. The "implementation" was a stub that satisfied the type system.

**The rule:** Every task in IMPLEMENTATION_PLAN.md MUST have a `<reality_test>` field alongside `<verify>`. The verify field checks compilation. The reality test checks non-trivial behavior that a stub CANNOT fake.

**The one-line test:** If a test can pass with `return vec![0.0; 1536]` or any hardcoded value, it is NOT a reality test.

**Reality test patterns:**

| Component type | Reality test pattern |
|---------------|---------------------|
| **Embedder** | Embed semantically similar pair ("cat slept", "feline rested") + dissimilar ("stock market crashed"). Assert: `distance(similar) < distance(dissimilar)`. Byte-hasher fails this. |
| **Reranker** | Fixed query + relevant doc + gibberish doc. Assert: relevant doc scores higher. Random/fixed scores fail this. |
| **Database/Graph** | Insert data → query it back → assert correct structure. Empty/fake DB fails this. |
| **HTTP server** | Start binary → `curl /endpoint` → assert response contains expected content. Unserved assets fail this. |
| **File watcher** | Modify file → assert watcher fires within timeout. No-op watcher fails this. |
| **CLI tool** | Run command → assert stdout contains expected output. Stub binary fails this. |

Task XML example:
```xml
<task id="M-003" req="REQ-002" wave="1" depends="W0-002">
  <description>LlamaCppEmbedder implementation</description>
  <verify>cargo check -p pythia-core</verify>
  <reality_test>Embed "fn main() {}" and "struct Color { r: u8 }" → 
    cosine distance > 0.1 (semantically different code). 
    Embed "fn get_user()" and "fn fetch_user()" → 
    cosine distance < 0.3 (semantically similar code).</reality_test>
</task>
```

---

## Phase 5: Build Execution

### Step 5.1: Execution Plan

Invoke `superpowers:writing-plans` on the IMPLEMENTATION_PLAN.md. This produces:
- XML task blocks with `<reality_test>` fields (mandatory)
- Wave ordering and dependency validation
- `policy-rules.yml` (enforcement rules)

**Every task MUST have a `<reality_test>`.** If the writing-plans step produces a task without one, the plan fails validation. Go back and add it.

### Step 5.2: Worktree Commit SHA Gate (CRYSTALLIZED — Pythia v2 2026-04-07)

**Origin:** Pythia v2 launched 5 Codex worktrees before Wave 0 was committed to git. All 5 built against v1 state. Deleted and restarted.

**The rule:** Before creating ANY worktree or dispatching ANY parallel agent:
1. Verify Wave 0 is committed: `git diff --quiet` (no uncommitted changes)
2. Record the commit SHA: `WAVE0_SHA=$(git rev-parse HEAD)`
3. Print: `Wave 0 committed at {WAVE0_SHA}. Worktrees will branch from this commit.`
4. After creating each worktree, verify: `git -C <worktree> rev-parse HEAD == $WAVE0_SHA`

If ANY check fails → **STOP.** Do not create worktrees. Commit first.

### Step 5.3: Build

Invoke `superpowers:executing-plans`. This:
- Executes tasks in wave order
- Runs `<verify>` after each task (compilation check)
- Runs `<reality_test>` after each task (stub detection)
- Appends to BUILD_MANIFEST.md after each completed task
- Appends to DEVIATION_LOG.md when plans change
- Stops at integration gates for manual verification

**If a reality test fails:** The task is NOT complete. The implementation is a stub. Fix it before proceeding. Do not mark it done and move on.

---

## Phase 7: Reality Gate (CRYSTALLIZED — Pythia v2 2026-04-07)

**Origin:** Pythia v2 passed Gate 1 ("search pipeline works") with stubs that produced fake vectors. `cargo test` passed because tests checked types, not behavior. The "gate" tested surrogates instead of reality.

After all integration gates pass, run the Reality Gate. This is a SEPARATE check from the test suite.

### Step 7.1: Stub Scan

For every implementation file produced during the build, search for:
- `todo!()`, `unimplemented!()`, `unreachable!()`
- Functions that return hardcoded values without touching external state (model files, databases, network)
- Test assertions that only check type/length/shape, not semantic correctness
- Any function whose body is < 5 lines implementing a trait method that should be complex

### Step 7.2: Reality Test Suite

Run ALL reality tests from the implementation plan. These are the `<reality_test>` fields, collected into a single test run.

```
═══════════════════════════════════════════
  REALITY GATE
═══════════════════════════════════════════

  Stub scan: [N] files scanned, [M] suspicious
  Reality tests: [N] passed, [M] failed

  ✅ M-003 LlamaCppEmbedder: semantic distance test PASS
  ✅ M-009 OnnxReranker: relevance ordering test PASS
  ❌ M-015 pythiad: GET / returns 404 (frontend not served)
  ...

  VERDICT: PASS / BLOCKED ([list failures])
═══════════════════════════════════════════
```

**Gate rule:** Reality Gate failures are BLOCKERS. A stub that passed `cargo test` but fails the reality test is NOT shipped. Fix the implementation.

---

## Phase 8: Post Rodeo Audit

Invoke `/postrodeo` on the completed build. This runs the full 6-phase audit:
1. Completion Matrix (REQ → test mapping)
2. Deviation Analysis (plan vs actual)
3. Git Forensics (commit analysis)
4. Twin Review (deviations reviewed by Gemini + Codex)
5. Layer 6 Semantic Logic Check (code review by twins)
6. Retrospective Report (RETROSPECTIVE.md written)

The postrodeo verdict determines ship-readiness:
- **SHIP** — all REQs pass, all reality tests pass, no findings
- **SHIP WITH ACKNOWLEDGMENTS** — all pass, findings acknowledged
- **BLOCKED** — REQs failing, stubs detected, unacknowledged findings

Only SHIP and SHIP WITH ACKNOWLEDGMENTS allow proceeding.

---

## The Full Pipeline

```
Phase 0    → User Story Validation (is the spec a product humans can use?)
Rounds 1-N → Architecture Review (is the design sound?)
Phase 3    → Post-Review Quality Gate (is the spec internally consistent?)
Phase 4    → Canonical Docs + Trait-to-Task Traceability
Phase 5    → Build Execution (with reality tests + worktree SHA gate)
  Gate 1   → Engine works (search pipeline)
  Gate 2   → Graph works (structural intelligence)
  Gate 3   → Full system works (all features)
Phase 7    → Reality Gate (are implementations real, not stubs?)
Phase 8    → Post Rodeo Audit (/postrodeo — full retrospective)
  ↓
SHIP or FIX
```

One command. One skill. Spec to ship. No steps to remember.
