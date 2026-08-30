# Pantheon Gates Rust Port — Round 3 Decision Ledger

**Spec:** `/Users/mikeboscia/projects/triumvirate/daemon/docs/specs/pantheon-gates-rust-port.md`
**Round:** 3 of projected 5-7
**Round scope:** Gate 0 concrete artifact authoring (YAMLs + pool), twin review synthesis, scope-closure torture
**Experimenter:** mike-boscia
**Round opened:** following Round 2 ledger ack
**Round closed:** 2026-04-21

---

## What Round 3 produced

**Gate artifacts committed to `/Users/mikeboscia/projects/triumvirate/gates/`:**
- `gate-0.1.yaml` — daemon + CLI dispatch, 3 hypotheses
- `gate-0.2.yaml` — daemon + commercial API, 3 hypotheses
- `gate-0.3.yaml` — HttpEndpoint + local MLX, 1 hypothesis
- `gate-0.4.yaml` — HttpEndpoint over LAN (Ollama, transport-focused POC), 1 hypothesis
- `prompts/gate-0-pool.yaml` — 30-prompt shared corpus, sha256:1ce30a26f5da8d57f6240e4566eba2d96a350112da690f873bd17b8507c4f8c1

**Spec delta:** REQs 064–086 filed; REQs 043, 052, 057, 058, 072, 074, 075 refined with Round 3 findings.

---

## Phase 1 — Gate 0 composition and shape

| ID | Decision | Option chosen | Ack mechanism |
|---|---|---|---|
| D-1 | Gate 0 structure | Four sub-gates (0.1 CLI → 0.2 commercial API → 0.3 local MLX → 0.4 LAN vLLM) staged ramp | "oh - because it has to be OpenAI compatible -- got it" |
| D-2 | Gate 0.1 fleet shape | Shape 1 — three-agent (claude/codex/gemini) | "Shape 1" |
| D-3 | Gate 0.2 providers | Three (OpenAI / Anthropic / Google) | "Three" |
| D-4 | Gate 0.3 model | Qwen2.5-0.5B-Instruct with 1.5B fallback; reframed as internal tooling / "toy" (not commercial — licensing concerns not applicable) | implicit ack |
| D-5 | Prompt pool scope | Option 1 — shared pool across all four sub-gates for cross-backend comparability (REQ-061) | implicit ack |
| D-6 | Gates framing | Three-purposes locked: validation + requirements forcing-function + commercial readiness proof | "yes — locked" |
| D-7 | Fleet entry shape | Option B — per-agent-backend, role coupled to agent | "B — also makes sense downstream..." |
| D-8 | Role semantics | Level 2 — metadata only, not enforced | "level 2" |
| D-9 | Warmup strategy | Option B + explicit warmup declaration | "Option B + explicit warmup" |

---

## Phase 2 — Gate 0.1 YAML authoring + revisions

| ID | Decision | Option chosen | Ack mechanism |
|---|---|---|---|
| D-10 | Gate 0.1 hypothesis approach B — concrete YAML draft | Drafted at `gates/gate-0.1.yaml` | "then show me that" |
| D-11 | Quicksearch-informed Gate 0.1 revisions (N=30 per run, backend-aware timeouts/warmup, median×1.2/p95×1.4/p99 alert-only, temperature=0, TTFT/TPOT when_measured) | All revisions applied | "Approve the revisions to the YAML" |
| D-12 | Twin dispatch pattern for Gate 0.1 review | Parallel — Gemini + Codex simultaneously | "paralell" |

---

## Phase 3 — Twin review synthesis and Gate 0.1 wiring

| ID | Decision | Option chosen | Ack mechanism |
|---|---|---|---|
| D-13 | Tier 1 vs Tier 2/3 twin-synthesis bucketing | Pivot to drafting prompt pool (option B of A-vs-B) to unlock content-fidelity fixes | "(B) Pivot to drafting gates/prompts/gate-0-pool.yaml next" |
| D-14 | Prompt pool size | Option C — expand to 30 prompts so num_tasks=30 draws without replacement | "C" |
| D-15 | Gate 0.1 wiring | Applied: prompt_pool_hash, fingerprint_overlay per hypothesis, response_contains_expected_phrase sanity | "apply both now" |
| D-16 | New primitives filed from wiring work | REQ-064 (expected_contains_any OR-match), REQ-065 (fingerprint_overlay) | "add these two REQs to the spec now" |

---

## Phase 4 — workload_shape + Gate 0.2/0.3/0.4 authoring

| ID | Decision | Option chosen | Ack mechanism |
|---|---|---|---|
| D-17 | Daemon vs http_api routing — honest type-system vs magic-string | Option B — `workload_shape:` field decouples Rust kind from workload profile | "B" |
| D-18 | Gate 0.2 YAML authoring | Authored at `gates/gate-0.2.yaml` with workload_shape: http_api, prerequisite_gates, secret-handling comments | implicit ack via flow continuation |
| D-19 | Additional primitives from Gate 0.2 | REQ-067 (prerequisite_gates data-driven ordering), REQ-068 (provider field decouples billing from agent routing) | "Write those two REQs now" |
| D-20 | Gate 0.3 authoring | HttpEndpoint + local MLX Qwen, single hypothesis | "continue" |
| D-21 | Gate 0.4 scope pivot | From multi-hypothesis vLLM gate to single-hypothesis Ollama transport-focused POC, perf targets all alert_only | reframed via user question "0.4 is testing a transport - right?" |
| D-22 | Pre-flight sequence for Gate 0.4 | Option A — approve 8-check sequence as drafted | "A" |
| D-23 | Pre-flight REQ filing | REQ-077 filed with eight checks and exit code 9 = PREFLIGHT_FAILED | completed as part of D-22 |

---

## Phase 5 — Full quartet twin review + Tier 1/2/3 closure

| ID | Decision | Option chosen | Ack mechanism |
|---|---|---|---|
| D-24 | Pre-ledger-close twin review posture | Option C — send full Gate 0 quartet + REQs 064–077 to both twins for adversarial review | "C" |
| D-25 | Tier 1 bug fixes from twin synthesis | Applied: policy-key mismatch fixed in all four YAMLs (fleet-entry-name now matches timeout lookup key), stale comment at Gate 0.1 line 114 corrected | "that order" |
| D-26 | Tier 2 refinements | Filed: REQ-078 (cost-rate staleness), REQ-079 (daemon internal-health pre-flight), REQ-080 (baseline anomalous-run detection); refined: REQ-072 (per-hypothesis fingerprint), REQ-074 (two-pass strict loader), REQ-075 (per-origin not per-host) | continuation of "that order" |
| D-27 | Tier 3 #1 — Gate 0.3→0.4 isolation is 2 variables not 1 | Option A — acknowledge via explicit scope-note in Gate 0.4 comments; triage protocol documented | "a" |
| D-28 | Tier 3 #2 — prompt pool capability scope for Gate 0.3/0.4 | Option B — declarative gate-level `prompt_pool_filter:`; twins unanimous | "B" (user) + twin unanimous endorsement |
| D-29 | Filter REQ + wiring | REQ-081 filed with minimum-viable grammar (`purposes_any` + `ids_any`), hash-before-filter preservation, zero-match CONFIG_ERROR, fingerprint_overlay_required guardrail; filter wired into Gate 0.3 and Gate 0.4 | "apply both now" |
| D-30 | Tier 3 #3 — baseline statistical rigor | Option C — swap 3σ outlier detection for MAD (median absolute deviation); keep variance-based stability gate for v1; defer bootstrap-CI to future follow-up if empirically needed | "C" (after explicit English re-explanation of what "right" statistical approach would cost) |
| D-31 | REQ-080 MAD hardening | REQ-080 edited in-place: 3σ mean+std-dev rule replaced with MAD-based robust outlier detection; edge case (MAD=0 on identical samples) handled explicitly; audit trail preserved in REQ prose | completed as part of D-30 |

---

## Phase 6 — Self-torture (items neither twin round surfaced)

| ID | Decision | Option chosen | Ack mechanism |
|---|---|---|---|
| D-32 | Tier-3 closure + self-torture pivot — three items (Tier 0 secrets, evidence bundle tree, hypothesis concurrency) — attack all, defer, or close? | Option A — torture all three in sequence before Round 3 close | "A" |
| D-33 | Tier 0 secrets spec — approach | Option A post-quicksearch — research-grounded tier ordering (ephemeral flag → env var → OS keyring → 1Password → file fallback), first-hit-wins default with strict-mode override, `secrecy` crate in-memory, extensibility point documented but deferred | "A" (after quicksearches) |
| D-33a | REQ filing | REQ-083 filed (credential resolution); REQ-052 extended (field-name redaction + deterministic tokenization + structural preservation); REQ-074 extended (Pass 1 credential-pattern detection at YAML load); Gate 0.2 comment updated to reference new tier numbering | continuation of D-33 |
| D-34 | Evidence bundle directory tree consolidation | REQ-084 filed with canonical layout consolidating REQ-034/041/045/047/048/052/054/057/058/061/071/072/073/076/077/079/080/081 layout directives; three structural calls locked: preflight inside cluster, baseline at top level, tokens-cumulative.jsonl at `~/.pantheon/` root | "let's get back to the scope" — implicit approval of defaults |
| D-35 | Hypothesis concurrency at Gate 0 | Option A — file both REQ-085 (Gate 0 serial hypotheses with intra-hypothesis concurrency to semaphore cap) and REQ-086 (concurrent-dispatch-as-validated-variable targets future Gate 2) | "A" |

---

## Phase 7 — Inbound external signal (parked)

| ID | Event | Disposition |
|---|---|---|
| D-36 | COL Todd L. Poindexter (CIO, XVIII Airborne Corps) email — warm intro to Army SW Engineer (CW3 Pilkington) + CDAO (Briant Higgins) + AQ/JIOP acquisition for sovereign agentic AI discussion | Parked as next workstream after Round 3 close. User directive: "we don't need to do any of this NOW - let's get back to the scope." Defense briefing stack (talking points → slide deck → leave-behind one-pager) to open post-ledger under `/Users/mikeboscia/projects/triumvirate/defense-brief/` (directory staged) |

---

## Live receipts (cross-provider peer review earning its keep)

Two concrete receipts surfaced during Round 3 where cross-vendor adversarial peer review produced findings same-provenance self-review would not have:

1. **Codex quartet-review caught a literal bug across all four YAMLs** (D-25) — fleet entry names did not match timeout map lookup keys. Four files, four misses, shipped past my own review. Would have caused runner undefined behavior at implementation time. Same-agent review did not catch this; cross-agent review did.

2. **Gemini quartet-review surfaced cost-rate staleness as a data-integrity defect** (D-26 → REQ-078). YAML-encoded per-MTok pricing goes stale silently; no mechanism caught this until adversarial review asked "how do you detect stale rates." Codex did not flag this (out of its implementation-focused attack surface); Gemini did.

Documented here to provide evidence for the Pantheon sovereign-AI thesis: cross-provider peer review is not theoretical differentiation; it is empirical defect-finding on this very project's artifacts.

---

## Outstanding items at Round 3 close

None. All surfaced decisions have been acked and filed.

## Items deferred to Round 4+

- Secondary REQs noted but not filed (low priority, recorded for traceability):
  - Prerequisite daemon sprint consolidation (REQ-043 `usage` + REQ-079 `/health` — one sprint or two?)
  - Future vLLM-on-Vulcan-1 gate roadmap (currently hand-waved as "deferred")
  - Baseline refresh UX — what happens when `--re-baseline` runs?
  - CLI surface spec — flags scattered across REQs, no single canonical list
  - FailureCategory completeness for `cli_subprocess` workload
- Round 4 scope itself (to be defined at Round 4 open)
- Defense briefing stack (parked per D-36)

---

## Round 3 statistics

- **REQs filed this round:** 23 new (REQs 064–086)
- **REQs refined this round:** 7 (043, 052, 057, 058, 072, 074, 075)
- **Gate YAMLs authored:** 4
- **Prompt pool size:** 30 prompts
- **Twin review exchanges:** 3 (quartet review, pool filter, statistical rigor)
- **Quicksearches executed:** 4 (pre-Gate-0.1 revisions) + 4 (secrets research)
- **Bugs caught by cross-provider peer review:** 2 (policy-key mismatch, cost-rate staleness)
- **User compound-question violations by Claude:** 1 (D-12 six-sub-decision burst — captured as memory `feedback_one_question_at_a_time.md`)
- **Hook blocks dogfooded:** 1 (credential pattern detection triggered on REQ-083 first write — same discipline REQ-074 is specified to enforce on operator-authored YAML)

---

**Ledger status:** Complete. Round 3 closed. Ready for user ack and Round 4 open (or pivot to defense briefing workstream).
