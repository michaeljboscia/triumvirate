---
name: council
description: Use when the user wants structured multi-model decision analysis for architecture, strategy, or risk decisions. Also use when the user mentions "council," "triumvirate decision," "get all three to weigh in," "structured decision," "decision record," or wants to analyze a strategic question from multiple AI model perspectives. NOT for debugging, implementation, or code review.
---

# Triumvirate Council — Multi-Model Decision Analysis

Dispatches the same question to Claude, Gemini, and Codex in parallel. Each model analyzes independently through its own analytical lens. Claude then synthesizes all three perspectives into a structured Architecture Decision Record written to disk.

## When to Use

- Architecture decisions (decompose vs. monolith, tech stack choice, schema design)
- Strategic direction (pivot, invest, cut, build vs. buy)
- Risk assessment before a big bet
- "Should we X or Y?" decisions with real consequences
- Any decision worth documenting for future reference

## When NOT to Use

| Need | Use Instead |
|------|-------------|
| Debugging | Direct work, `/plan`, or `our-systematic-debugging` |
| Implementation | Direct work or `/plan` |
| Code review | Twin review via `inter-agent-protocol` |
| Deep codebase research | Gemini oracle pattern |
| Quick opinion | Just ask — no council overhead needed |
| Recurring/automated tasks | Direct execution |

## How to Use

```
/council Should we decompose the correlation engine or keep it monolithic?
/council What's the right persistence layer — Supabase, DuckDB, or flat files?
/council Should we build the narrative generator as a Prefect flow or a standalone service?
```

If the question is too thin, the council will ask for goal, constraints, known facts, and unknowns before proceeding.

## What You Get

A decision record at `~/projects/<project>/decisions/YYYY-MM-DD_topic.md`:

| Section | Purpose |
|---------|---------|
| **Context & Forces** | What triggered this, hard constraints |
| **Decision Criteria** | Explicit criteria in priority order |
| **Verdict** | Actionable bottom line (1-3 sentences) |
| **Consensus** | Where all three models independently agreed |
| **Key Tensions** | Structured disagreements with severity markers |
| **Blind Spots** | What nobody addressed |
| **Confidence Map** | Where analysis is solid vs. shaky |
| **Alternatives Considered** | Options evaluated, why rejected |
| **Accepted Trade-offs** | Pain we're choosing to live with |
| **Assumptions** | What must remain true |
| **Signals to Revisit** | When to reconsider this decision |
| **Raw Perspectives** | Full model analyses in collapsible sections |

## Design Principles

1. **Architectural diversity > prompted diversity.** Three different model architectures produce genuinely different analyses. No fake personas.
2. **Synthesis isolation.** Claude writes its position before reading siblings. Synthesizes as Neutral Arbiter, not participant.
3. **Model-specific lenses.** Gemini focuses on scale/ops/system health. Codex focuses on implementation/feasibility. Claude focuses on architecture/DX.
4. **Raw file access.** Siblings read files themselves — Claude sends file paths, not summaries.
5. **Minority report protection.** A 2v1 split elevates the dissent, not buries it.
6. **Decision records are permanent.** Written to disk, referenceable, searchable.
