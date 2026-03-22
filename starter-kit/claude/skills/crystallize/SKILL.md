---
name: crystallize
description: Use when encountering recurring failure patterns, after 3+ failures on the same problem class, or when explicitly asked to crystallize a failure into a skill or skill matrix
---

# Failure Crystallizer

A skill matrix factory. Takes recurring failure patterns and produces skills or skill matrices with anti-rationalization enforcement.

## Entry Path Routing

Determine your entry point, then follow the corresponding path:

| How You Got Here | Start At | Why |
|---|---|---|
| From our-systematic-debugging Phase 5 | Phase 2 (read `factory/phase-2-diagnose.md`) | Failure data already captured |
| Manual `/crystallize` invocation | Phase 1 (read `factory/phase-1-capture.md`) | No prior context — interview user |
| Update existing skill/matrix | Edit directly | Add anti-rationalization entry to enforcement, add evidence to reference |

## Operating Modes (for produced matrices)

When building a matrix, determine which mode applies:

| Mode | When | Claude's Role |
|---|---|---|
| **Build** | Working on pipeline code | Infrastructure engineer |
| **Execute** | Pipeline exists and works | Operator — invoke and review |
| **Direct** | Rules codified, pipeline not deployed or underperforming | Constrained writer — enforcement + validation still apply |

## Pipeline Overview

Read `enforcement.md` for the 6-phase pipeline. Each phase has its own file in `factory/`.

## Triumvirate Rule

**All three agents. All conclusion-producing phases. No exceptions.**

Phases 2 (Diagnose), 3 (Graduate), and 6 (Validate) require triumvirate dispatch.
Phase 1 (Capture) is single-agent interview.
Phases 4 and 5 are construction — single agent with user collaboration.

Disagreement: majority vote (2 of 3). All three disagree → surface to user.

## Validation

Read `validation.md` for the crystallizer's own checklists (skills and matrices).

## Not Everything Gets Crystallized

This is for the shit that won't die. NOT for one-off failures. The user decides the threshold.
