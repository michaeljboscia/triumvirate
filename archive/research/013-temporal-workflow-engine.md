# Research 013: Temporal.io — Durable Execution for Agent Workflows

**Both twins proposed Temporal independently. This is the orchestration layer.**

## What Temporal Does
- Durable workflow execution — if step 8 fails, resume from step 8 (not step 1)
- Automatic retries with exponential backoff
- Saga pattern for compensation (rollback on failure)
- Human-in-the-loop signals (pause workflow, wait for approval)
- Visibility UI for workflow progress
- Go SDK is a primary supported language

## Why This Solves Our Problems
Current inter-agent: shell out → hope for response → timeout → retry from scratch → give up.
With Temporal:
1. `DebateWorkflow` — submit claim → wait for challenges → collect rebuttals → vote → decide
2. `SpecToPRWorkflow` — spec review → plan → implement → test → PR — any step can retry
3. `IncidentResponseWorkflow` — detect failure → diagnose → fix → verify — with compensation

## Integration with Temporal Go SDK
- Workflows defined as Go functions
- Activities = individual agent calls (Claude, Gemini, Codex)
- Activity retry policies: max attempts, backoff, timeout per activity
- Workflow visibility: see exactly where each debate/task is stuck
- Already integrates with OpenAI Agents SDK + Vercel AI SDK

## Key Insight
"Temporal bridges the gap between fragile AI prototypes and scalable, resilient production systems." This is EXACTLY our problem.

## Sources
temporal.io (extensive), dev.to, github.com, medium.com, youtube.com
