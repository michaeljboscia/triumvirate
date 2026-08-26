# Raw peer output: gate-0-plumbing.md unit 2 (lines 112-245)

Step 3 compose stack, Step 4 H-0.1 health check, Step 5 H-0.2 dispatch test.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

- **Step 3 is not executable as written.** Both generated files contain only `# [paste content from above]` at lines 175-182. `docker compose up` (185) will fail or start nothing meaningful.

- **Local collision risk is real.** Fixed host ports `4222`, `8222`, `8000`, `7788` are bound at 123, 133, 147. **On a persistent local box, NATS, vLLM/Ollama-style services, dev servers, or prior Triumvirate runs can already occupy these. No project name, no preflight port check, no cleanup.**

- **Service definitions partly coherent.** `triumvirate` correctly uses Docker DNS names for NATS and mock-vLLM (161-166) and mounts `/tmp/config` into `/etc/triumvirate` because `cd /tmp` precedes compose (184). **But the healthchecks assume `wget` exists in the NATS image and `curl` exists in the harness/Triumvirate images** (125, 135, 152). That may fail independent of service health.

- **`depends_on` (148-150) waits for container healthchecks, not real application readiness**, and does not protect Step 4 from premature execution because line 188 uses a blind `sleep 15`.

- **GCP assumptions remain in image names:** Artifact Registry paths at 121, 131, 141, plus `$REGISTRY/pantheon-test-harness:main` at 223. Local needs real local tags or a registry mapping.

- **H-0.1 asserts HTTP reachability only.** Lines 196, 199, 202 define "healthy" as successful HTTP responses from host ports; evidence at 208-210 records only OK/FAIL from those same curls. **It can pass while the service is minimally up but not functionally able to dispatch, connect to NATS, or run inference.**

- **H-0.2 proves completion, not correctness.** Asserted output is `"tasks_completed": 5`, `"tasks_errored": 0`, `"verdict": "PASS"` (239-242). **An empty or malformed task result could still pass if the harness counts it as completed.** Minimum fix: expected-output assertions per canned task, comparing response body/schema/content against fixtures, failing if any result is empty, malformed, or semantically wrong.

- **Mock vLLM** is the harness image running `--mode=mock-vllm-server` (130-136). Its only specified behavior is that `/v1/models` returns something curlable. Sufficient to validate HTTP routing into an OpenAI-compatible shape, **but it hides the likely breakpoints: model loading, GPU/runtime compatibility, request schema differences, streaming, latency, memory pressure, real inference failures.**

- **Timeouts are weak.** Healthchecks have bounded probes (126-128, 136-138, 153-155), but orchestration uses a fixed `sleep 15` (188) instead of waiting deterministically. **Dispatch (218-227) has no visible timeout; if the harness or Triumvirate blocks, the command hangs.**

- **Commands likely to fail today:** compose creation (175-182, placeholders); image pulls (121, 131, 141, 223) if tags do not exist or auth is absent; healthchecks if images lack `wget`/`curl`; evidence write (211) if `/tmp/evidence/$RUN_ID` does not exist or `RUN_ID` is unset.

---

## GEMINI (strategic angle)

**1. Mocking is the correct approach.** Gate 0 exists to isolate orchestration plumbing from inference unpredictability. **A passing Gate 0 proves only that Docker networking, NATS messaging, and Triumvirate configuration (119-156) communicate successfully. It licenses zero confidence in GPU allocation, CUDA drivers, or real vLLM stability.**

**2. Round-trip is correct for plumbing, but line 241 wrongly introduces a performance assertion.**
`"round_trip_median_ms": 45`. **A plumbing test must assert strict deterministic payload delivery and correct routing. It must explicitly avoid asserting latency or output semantics.**

**3. State contamination on a persistent box, three sites:**
- **175-182:** configs written to `/tmp/docker-compose.gate-0.yml` and `/tmp/config/gate-0.toml` persist.
- **185:** `docker compose up -d` leaves container, network, and port-mapping state running indefinitely if not stopped.
- **211, 227-229:** `/tmp/evidence/$RUN_ID` persists permanently. **If `$RUN_ID` generation fails or collides, evidence is contaminated by prior runs.**

**4. Missing entirely: an explicit teardown step.** No `docker compose down -v`, no cleanup of `/tmp` configs or evidence. **Without enforced teardown, the system guarantees state contamination on a persistent box.**

**5. The 45-minute bound must remain.** On GCP it prevented runaway billing. **Locally it is required to stop a hung process indefinitely squatting critical ports (4222, 7788, 8000) and blocking all future runs.** Replace with a strict runner timeout coupled to an unconditional teardown hook.

---

## DEEPSEEK (adversarial logic angle)

Asked to resolve a direct contradiction: Codex says assert output correctness; Gemini says explicitly do not assert output semantics.

> **Rule:** In a plumbing test with a mocked component, assert that each task's **routing envelope** (identity,
> headers, source, destination, trace context, and any deterministic control fields) arrives intact and is delivered
> to the right stage, but do **not** assert on the semantics or correctness of the mock's output payload.
>
> **Belongs in the test:** assert that task `#3` entered the mocked inference stage with request ID `req-3` and that
> the orchestrator routed the corresponding response back to the outbound topic with the same correlation ID,
> **regardless of whether the content is `{"ok":true}` or `"malformed"`.**
>
> **Does not belong:** asserting the mocked response is semantically correct, e.g. that `sum([2,2])` returned `4`,
> **because that tests the mock's hardcoded behavior, not the plumbing.**

**Both reviewers were right about different things, and the boundary is the envelope.** Codex was right that
`tasks_completed: 5` is too weak, because it proves nothing about whether payloads arrived intact or were routed
correctly. Gemini was right that asserting the mock's output semantics tests the mock rather than the pipeline.

**The correct assertion is neither "5 completed" nor "the answer was right." It is: every task's correlation ID
round-tripped, each reached the stage it should have, and the envelope was not corrupted in transit.** That is
strictly stronger than the current test and strictly narrower than checking answers.
