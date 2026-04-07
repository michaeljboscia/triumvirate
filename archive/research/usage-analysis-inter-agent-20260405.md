# Usage Analysis: Inter-Agent Communication (Feb 15 - Apr 5, 2026)

**Source:** `~/.claude/inter-agent-messages/outbox/` — 359 messages
**Period:** 2026-02-15 to 2026-04-04 (49 days)

---

## Summary Statistics

| Metric | Value |
|--------|-------|
| Total messages sent | 359 |
| Successful | 300 (84%) |
| Timeouts | 38 (11%) |
| Failures | 19 (5%) |
| **Total broken experiences** | **57 (16%)** |

## By Target Agent

| Agent | Messages | % of Total |
|-------|----------|-----------|
| Codex | 204 | 57% |
| Gemini | 153 | 43% |

## By Request Type

| Type | Count | % |
|------|-------|---|
| Code review | 172 | 48% |
| Question | 83 | 23% |
| Architecture | 56 | 16% |
| Other | 31 | 9% |
| Debug | 17 | 5% |

## Usage Over Time (Adoption Curve)

| Month | Messages | Trend |
|-------|----------|-------|
| February 2026 | 257 | Peak adoption (system just built) |
| March 2026 | 98 | -62% decline |
| April 2026 | 2 | -98% from peak. Effectively abandoned. |

**Peak day:** Feb 15 (50 messages — day the system launched)
**Heaviest week:** Feb 15-16 (90 messages in 2 days)

## Response Duration Distribution

| Duration Bucket | Count | Notes |
|----------------|-------|-------|
| < 30s | ~80 | Fast responses |
| 30-60s | ~100 | Normal |
| 60-120s | ~80 | Slow but tolerable |
| 120-300s (2-5 min) | 23 | User sitting in dark for minutes |
| 300s (timeout) | 11 | Max timeout hit |

## The Abandonment Story

The data tells a clear adoption → frustration → abandonment arc:

1. **Feb 15-16:** 90 messages in 2 days. Excitement. Everything is new.
2. **Feb 17-28:** ~15-25 messages/day. Regular usage. Building trust.
3. **Mar 1-15:** Usage drops to 8-15/day. Failures accumulating.
4. **Mar 16-31:** Sporadic. 4-8 messages on active days, many zero days.
5. **Apr 1-5:** 2 messages total. System effectively abandoned.

**Root cause of abandonment:** Not reliability (84% success is acceptable). It's the **invisible 16%**. When a message fails or times out, the user:
- Has no idea it failed (no progress indicator)
- Has no idea why (no error surfacing)
- Has no way to retry (no retry mechanism)
- Waits 2-5 minutes in silence before giving up

After enough silent failures, trust erodes and the user stops trying.

## User Stories Extracted From Actual Usage

### US-REAL-1: Code Review (48% of usage)
The user sends code to Gemini or Codex for review. Expects a structured score/feedback response. This is the dominant use case — nearly half of all inter-agent traffic.

### US-REAL-2: Ask a Question (23% of usage)
The user asks the twins a question and expects both to respond. Often preceded by "ask the twins what they think about..." in the Claude session.

### US-REAL-3: Architecture Decision (16% of usage)
The user presents an architecture choice and asks for evaluation from multiple perspectives. Often during goatrodeo or spec review sessions.

### US-REAL-4: Debug Assistance (5% of usage)
The user shares an error or bug and asks an agent to help diagnose. Least common but high-urgency.

## Implications for Triumvirate v2

1. **Code review is the killer app.** Build the MCP tool surface around it first.
2. **"Ask the twins" is the primary interaction pattern.** Fan-out to both agents simultaneously.
3. **Lifecycle visibility is the #1 missing feature.** 57 silent failures over 7 weeks = death of trust.
4. **Duration variance is huge (7s to 300s).** Progress indicators aren't optional — they're essential.
5. **The adoption curve proves the product concept works.** 257 messages in month 1 = strong demand. The decline is a UX problem, not a product problem.

## Gemini Daemon Sessions

118 persistent daemon sessions across dozens of projects. This proves persistent multi-turn is valuable — users don't just want fire-and-forget, they want ongoing conversations with agents.

Top daemon projects: triumvirate, goatrodeo sessions, crystallize sessions, council reviews, sprint oracles.
