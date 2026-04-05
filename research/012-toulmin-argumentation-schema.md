# Research 012: Toulmin Model — Structured Argumentation for Agent Debate

**This replaces "just give your opinion" with enforceable structured debate.**

## The 6 Components (JSON Schema)
```json
{
  "claim": "The main assertion",
  "data": ["Evidence supporting the claim"],
  "warrant": "The principle connecting data to claim",
  "backing": ["Support for the warrant itself"],
  "qualifier": "Strength indicator (probably, usually, certainly)",
  "rebuttal": ["Counterarguments and exceptions"]
}
```

## Why This Matters for Triumvirate
1. **Enforces structure** — agents can't just say "I think X." Must provide data + warrant
2. **Explainability** — human can trace reasoning: claim → data → warrant
3. **Debate mechanics** — agent B can specifically attack agent A's warrant, not just disagree
4. **Hallucination reduction** — requiring data forces grounding in evidence
5. **Argument strength scoring** — presence/quality of each component = measurable quality
6. **Machine-parseable** — JSON schema means the Go daemon can track, score, and arbitrate

## How to Implement
- Every agent response in debate mode must conform to Toulmin JSON schema
- Go daemon validates schema before accepting claims
- Missing warrant = claim rejected
- Missing rebuttal = claim flagged as overconfident
- Backing must reference verifiable source (file, git commit, API response)

## Sources
academy4sc.org, purdue.edu, ciris.info, mdpi.com, frontiersin.org, medium.com, promptlayer.com
