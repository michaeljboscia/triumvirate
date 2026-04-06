# Spec Kit Analysis (github/spec-kit)

**Date:** 2026-04-05
**Stars:** 85K | **Language:** Python | **License:** MIT

## What It Is

A CLI toolkit (`specify`) that enforces a phased spec-driven development workflow: **constitution -> specify -> plan -> tasks -> implement**. Works with 25+ AI agents (Claude Code, Copilot, Gemini, Codex, etc.) via slash commands.

## Core Mechanisms

**1. Constitution as Law.** A project-level constitution file (`/memory/constitution.md`) defines non-negotiable principles. The `plan` and `analyze` commands validate artifacts against it. Constitution conflicts are auto-CRITICAL. This is their enforcement of "what and why before how."

**2. Spec -> Plan separation.** `/speckit.specify` takes natural language and produces a spec with prioritized user stories (P1/P2/P3), Given/When/Then acceptance scenarios, and edge cases. NO tech stack allowed yet. `/speckit.plan` is a separate phase where architecture choices happen AGAINST the spec.

**3. Cross-artifact analysis (`/speckit.analyze`).** Read-only pass that builds semantic models from spec.md + plan.md + tasks.md and runs four detection passes: duplication, ambiguity (flags vague words like "fast" or "scalable"), underspecification (verbs without measurable outcomes), and constitution alignment. Capped at 50 findings. This is the AI-driven validation.

**4. Checklist as "unit tests for English."** `/speckit.checklist` generates domain-specific checklists that validate requirements QUALITY — not implementation correctness. Tests whether specs are complete, clear, and unambiguous.

**5. Extension hooks.** Before/after hooks on every phase via `.specify/extensions.yml`. Mandatory hooks block progression; optional hooks are offered. Enables gates like "Plan Review Gate" (requires PR merge before task generation).

## What We Could Adopt

1. **Constitution-first gate.** Formalize project principles before any spec work. Our `/goatrodeo` already pressure-tests specs, but a persistent constitution that auto-validates against every plan would catch drift earlier.

2. **Analyze pass.** The four-detection-pass pattern (duplication, ambiguity, underspecification, constitution alignment) maps cleanly to a post-goatrodeo quality gate. We could add this as a verification step in our superpowers planning flow.

3. **"Unit tests for English" checklist.** Domain-specific requirements quality validation is a gap in our process. We test code but not spec language. The CHK-ID numbering and append-only pattern is simple to adopt.

4. **Strict spec/plan separation.** We sometimes blend "what" and "how" in the same goatrodeo round. Enforcing a hard gate between specification (what+why) and planning (how+stack) would improve spec quality.

5. **Hook system for phase gates.** Our superpowers already has wave-based execution, but adding before/after hooks per phase would let us plug in mandatory reviews (e.g., inter-agent peer review before implementation starts).
