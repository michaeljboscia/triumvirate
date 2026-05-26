# Round 1 — Interrogation (ruthless-interrogator output)

Tags: `[RESEARCH]` = needs live Gemini search · `[INTERNAL]` = answerable from code/architecture ·
`[USER]` = genuine "what to build" decision for the operator.

## Fork A — Access & the no-API-keys rule
- **Q1 (REQ-DS-003) [USER]** — What is the *actual rationale* behind "subscriptions only, never
  API keys"? (a) cost/no-metered-runaway, (b) data privacy (API ToS ≠ subscription ToS),
  (c) sovereignty/airgap ethos, (d) credential-sprawl avoidance? The rationale decides whether
  DeepSeek's API-key path violates the *spirit* or only the *letter*. If privacy/sovereignty →
  self-host is the only compliant path. If cost → a spend cap satisfies it.
- **Q2 (REQ-DS-002) [USER]** — Is `vulcan-1` actually built and serving, or a plan? Choosing
  self-host as the access path makes the ENTIRE integration depend on hardware. Is DeepSeek
  blocked on vulcan-1, or do we ship cloud-API first and treat access as a swappable boundary?
- **Q3 (REQ-DS-002, REQ-DS-004) [RESEARCH]** — All three access options expose an
  **OpenAI-compatible** endpoint (cloud API, local vLLM/ollama, CLI-proxy). Is the access
  decision therefore a **false fork** at the integration layer — build against the
  OpenAI-compatible HTTP contract once, and access becomes a `base_url`+auth config choice
  (deferrable)? Verify how complete DeepSeek's OpenAI-compat actually is.

## Fork B — Integration shape
- **Q4 (REQ-DS-004) [RESEARCH/INTERNAL]** — DeepSeek has no first-party CLI. What does the
  subprocess path *buy* that native HTTP doesn't? Wrapping aider/OpenCode imports a full
  coding-agent's prompt scaffolding + file-editing behavior + sandbox needs for what could be a
  ~50-line HTTP call. Is "subprocess precedent" actually a liability here?
- **Q5 (REQ-DS-004 × REQ-DS-005) [INTERNAL]** — Shape is downstream of role: reasoning/Q&A →
  native HTTP, no sandbox; coding-writes-files → coding CLI subprocess + sandbox. Is the spec's
  fork ordering wrong (B can't be decided before C)?

## Fork C — Model & role
- **Q6 (REQ-DS-005) [USER/INTERNAL]** — What gap does DeepSeek fill? Codex owns coding; Gemini
  owns general + big context + search. The adversarial-diversity thesis is strongest for
  **reasoning** (R1 fails differently on logic/math). "Another coding agent" competes with
  Codex on its turf with a weaker tool story; "another general agent" competes with Gemini.
  Does the diversity thesis *force* role = reasoning specialist (R1)?
- **Q7 (REQ-DS-005) [RESEARCH]** — R1 "exposes its thinking." Does `ask_agent`'s plain-text
  contract handle a model emitting `<think>…</think>` (or `reasoning_content`) blocks? Is the
  trace separated from the answer, or does it pollute the consult output?

## Resilience
- **Q8 (REQ-DS-006) [INTERNAL/RESEARCH]** — `agy_resilience` was built for a SUBSCRIPTION/quota
  model (token-bucket vs quota pool, reset-window cooldown aligned to quota reset). A metered
  pay-per-token API has different failure economics: RPM/TPM rate limits + a spend ceiling, no
  "quota reset window." Does the reset-window-cooldown concept even map? Does the breaker need a
  NEW spend-based trip condition, not just a 429 condition?
- **Q9 (REQ-DS-007) [INTERNAL]** — "SIGKILL-timeout" presupposes a subprocess to kill. Native
  HTTP has no process — it's a tokio request timeout + connection abort. Restate REQ-DS-007
  shape-agnostically.

## Degraded route
- **Q10 (REQ-DS-008) [USER/INTERNAL]** — "Route to another sibling" on failure silently
  substitutes a DIFFERENT model's answer when the user asked for DeepSeek's — defeating the
  diversity purpose AND violating "no silent failure." Is route-to-sibling ever acceptable, or
  must a diversity-motivated seat fail LOUD (the only honest option)?

## Token economics
- **Q11 (REQ-DS-009) [RESEARCH]** — DeepSeek API has CACHE pricing (cache hits ~10× cheaper),
  returned as separate token counts. Does `price_table`/`calculate_cost_usd` support tiered
  pricing (cache-hit vs cache-miss input)? If not, "exact" will OVER-bill cache-hit tokens at
  the miss rate. Is "exact" actually exact?
- **Q12 (REQ-DS-010) [INTERNAL/USER]** — Spend cap enforced WHERE (client-side cumulative-spend
  breaker reading the ledger, vs DeepSeek account limit)? What window (day/session/lifetime)?
  Hard-fail or warn at cap?

## Routing
- **Q13 (REQ-DS-011) [INTERNAL]** — Does sibling auto-routing even EXIST today? (Claude
  explicitly picks who to consult; grep shows no router.) "Auto-route to DeepSeek" may be
  inventing a router that isn't there. Default = explicit-only unless a router exists.

## Sandbox / verification / scope
- **Q14 (REQ-DS-012) [INTERNAL]** — Sandbox condition is fully determined by Fork C. Restate:
  sandbox IFF role implies file writes. (Moot if role = reasoning.)
- **Q15 (REQ-DS-017) [INTERNAL]** — What must the probe battery prove, against which endpoint,
  given access is undecided? Probe the cloud API (free 5M grant) to validate OpenAI-compat +
  real token counts + failure signals regardless, then re-probe local if self-host wins?
- **Q16 (REQ-DS-001) [USER/INTERNAL]** — Grep confirms agy is a hidden `GeminiBackend`, NOT in
  `is_supported_agent_name`. First-class DeepSeek is MORE invasive than agy was (new gate entry,
  display name, prewarm, /status). Is full first-class the only coherent option, or is there a
  lighter registration that still exposes `deepseek` via `ask_agent`?
- **Q17 (all) [USER]** — What is the MINIMUM shippable v1? Candidate floor: "consult R1 via
  cloud API, reasoning role, fail-loud, metered-exact, no sandbox, explicit-route-only."
  Everything else (self-host, coding role, auto-route, degraded-to-sibling) = deferred
  expansion. Should the spec define a v1 floor and defer the rest?
