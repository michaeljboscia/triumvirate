# Raw peer output — 10-PREFLIGHT.md Phase 3 (lines 273-371)

Verbatim agent responses. Synthesized findings in `../REVIEW-PROGRESS.md`.

**Date:** 2026-08-25
**Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Step 3.1 image tags**

`vllm/vllm-openai:v0.6.5` on line 283 still resolves upstream: Docker Hub tag API returned `200`.

`vllm/vllm-openai:v0.6.5-cpu` on line 288 does **not** resolve: Docker Hub tag API returned `404`. vLLM appears to publish CPU images under a separate repo, `vllm/vllm-openai-cpu`, not as `-cpu` tags on `vllm/vllm-openai`; the separate CPU repo exists, but `vllm/vllm-openai-cpu:v0.6.5` also returned `404`.

`nats:2.10-alpine` on line 293 still resolves upstream: Docker Hub tag API returned `200`. Multi-platform.

Pinning vLLM `v0.6.5` is not defensible for current validation unless the test intentionally targets that historical runtime. Current vLLM release metadata shows much newer release lines and active model/runtime changes.

**2. Step 3.3 cannot build. Confirmed.**

Documented build context is `docs/pantheon/gcp-test-plan` at lines 365-366.

- Line 366: `-f harness/Dockerfile` fails, that file does not exist.
- Line 340: `COPY requirements.txt .` looks for `gcp-test-plan/requirements.txt`, but line 349 documents `harness/requirements.txt`; neither exists.
- Line 343: `COPY harness/ ./harness/` copies the existing shell/Python utility directory, but it is not a Python package and has no `harness.runner`.
- Line 344: `COPY fixtures/ ./fixtures/` fails, directory has never existed.
- Line 346: entrypoint fails at runtime, `harness.runner` does not exist.

Minimal Dockerfile for the harness that ACTUALLY exists:

```dockerfile
FROM google/cloud-sdk:slim

RUN apt-get update && apt-get install -y \
    curl git jq xxd \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY harness/ ./harness/

ENTRYPOINT ["/app/harness/runner-wrapper.sh"]
```

**3. `harness.runner` entrypoint (line 346) is fictional today.**

To make it real: `harness/__init__.py`, `harness/runner.py`, a CLI accepting the gate/config semantics the runbooks expect, and a dependency file at the path the Dockerfile actually copies.

It likely duplicates the existing `harness/runner-wrapper.sh`, which already owns provision, run, capture, destroy, evidence upload, and gate config loading. **The cleaner fix is to containerize the wrapper, not invent a parallel Python runner** unless there is a deliberate migration plan.

**4. Step 3.2 Cloud Build. Confirmed local failures:**

- Line 300 captions `triumvirate/cloudbuild.yaml`, but `/Users/michaelboscia/projects/triumvirate/cloudbuild.yaml` does not exist.
- Line 308 references `daemon/Dockerfile`, which does not exist.
- `gcp-test-plan/cloudbuild.yaml` also does not exist.

Line 318 is wrong as written. `${DEFAULT_REGION}` is not a built-in Cloud Build substitution. `${PROJECT_ID}` is built-in, but relying on shell-style env expansion inside a user-defined substitution value is the wrong mental model. Robust version:

```bash
gcloud builds submit \
  --config=cloudbuild.yaml \
  --substitutions=_REGISTRY="${DEFAULT_REGION}-docker.pkg.dev/${PROJECT_ID}/pantheon-images",SHORT_SHA="$(git rev-parse --short HEAD)"
```

`SHORT_SHA` is not reliably populated for `gcloud builds submit` from a local source upload. Google documents that unavailable default substitutions are replaced with empty strings.

**5. Cloud Build logging (line 320).** `CLOUD_LOGGING_ONLY` is defensible if using a user-specified service account: such builds must specify `logsBucket`, use `CLOUD_LOGGING_ONLY`, or use `NONE`. With no custom SA it is not inherently required. No separate logs bucket needed with `CLOUD_LOGGING_ONLY`.

**6. Cost claim is materially incomplete.** Cloud Build itself is plausibly low cost on default `e2-standard-2` pools with a free tier. But the header mentions only Cloud Build and ignores Artifact Registry storage. vLLM GPU images run roughly 8-10 GB compressed, and Artifact Registry charges storage above the free allowance.

**7. Other first-run failures**
- Line 280 assumes `DEFAULT_REGION` and `PROJECT_ID` are already exported; otherwise `REGISTRY` is malformed.
- Lines 283-295 assume local Docker exists and is authenticated to Artifact Registry.
- Lines 285, 290, 295, 367 assume the `pantheon-images` repo already exists.
- Line 325 fails, `cloudbuild.yaml` absent.
- Lines 310, 315 may produce an invalid/empty tag suffix if `SHORT_SHA` is empty.
- Line 366 depends on `${REGISTRY}` persisting from Step 3.1; a fresh shell breaks the tag.

**Net:** Phase 3 does not execute as written. Step 3.1 is partly valid except the vLLM CPU image. Step 3.2 is blocked by missing files and brittle substitutions. Step 3.3 is non-real.

---

## GEMINI (strategic angle)

**1. Artifact Registry vs Local Execution.** Pre-baking into GCP Artifact Registry (lines 279-296, 318, 366-368) no longer makes sense for Track A. With the first executor a local RTX 4000 Ada box, pushing to GCP only to pull back down introduces latency, egress cost, and invalidates an isolated testing topology. Build, tag, and retain in the local Docker daemon cache. Defer Artifact Registry until workloads actually migrate to GCP.

**2. The Air-Gap Implication.** Gate 6 claims no egress, but relying on Artifact Registry for runtime image pulls guarantees egress. Enabling PGA to reach Artifact Registry creates an outbound path. To produce credible client-facing evidence, images must be loaded from local OCI tarballs (`docker load`) or a strictly local registry before the network is severed.

**3. Supply Chain and Sovereignty.** Raw `docker pull` from Docker Hub (lines 283, 288, 293) is incompatible with a sovereign product claim.
- **No digest pinning:** tags like `v0.6.5` and `2.10-alpine` are mutable; pin by `@sha256:...`.
- **No verification:** no signature check (Cosign) against upstream tampering.
- **No SBOM/scanning:** no bill of materials, no vulnerability scan before execution.

A client security team evaluating sovereignty will reject a pipeline that trusts unverified, unpinned binaries from a public registry.

**4. vLLM v0.6.5 Pinning is a fatal error in August 2026.**
- Lacks optimizations and possibly support for the local Ada Lovelace (sm_89) card and GCP Blackwell.
- Misses nearly two years of FlashAttention, continuous batching, FP8/AWQ quantization, and newer model architectures.

**5. Cuts and Deferrals**
- **vLLM CPU image (cut):** lines 287-290. With a local Ada GPU available, a CPU image for plumbing tests is waste.
- **Cloud Build Triumvirate image (cut):** lines 300-326. Submitting to Cloud Build to pull back locally makes no sense; build locally.
- **Test harness image (cut):** lines 328-369. Line 344 copies a directory that does not exist, and containerizing a local plumbing harness is over-engineered. Run it via `uv` or a venv.
- **GCP pushes (deferred):** lines 285, 290, 295, 367.

Only **vLLM GPU** and **NATS** earn their place for Track A, both pinned by digest and pulled to the local machine.

---

## DEEPSEEK (adversarial logic angle)

Asked one focused question: does the build-time supply chain defeat the runtime isolation claim?

> Short answer: yes, it defeats the isolation claim, not just a separate concern.
>
> The runtime egress test only proves that this workload, in that test window, didn't emit outbound packets. It cannot
> inspect the contents of the images. Because the images were pulled from public mutable tags with no digest pinning,
> no signatures, and no SBOM, they are effectively **untrusted executable content**.
>
> Class of thing the test structurally can't detect:
>
> - **Dormant/triggered malicious code**: logic bombs, time-delayed callbacks, backdoors that wait for a specific
>   signal or input.
> - **Pre-positioned exfiltration code**: an agent that stays quiet during the test but exfiltrates later.
> - **Exploitable vulnerable components**: malicious or vulnerable dependencies that only compromise the system when
>   some future interaction happens.
> - **Internal pivot tooling**: code that does nothing outbound, but attacks other internal services once running.
>
> Minimum evidence to close the supply-chain gap:
>
> - **Pin every image by digest** (not mutable tag).
> - **Verify signatures** at pull time (cosign / Notary).
> - **Generate and scan an SBOM** for every image.
> - **Scan for known vulnerabilities** in those artifacts.
> - **Record provenance/attestation**: build command, source repo/commit, dependency tree per image layer.
>
> With that, the runtime egress test can remain as one control, but it is not sufficient evidence of isolation on its own.

**Two-peer convergence with Gemini.** Both independently concluded that unpinned public-registry pulls are
incompatible with any isolation or sovereignty claim, and both named the same remedy set (digest pinning, signature
verification, SBOM, provenance). Gemini reached it from the client security team's perspective, DeepSeek from what the
test is structurally capable of observing.

**Note this survives the owner's terminology correction.** Even under the working definition of air-gap as "not on the
public internet," a dormant callback baked into an image at build time is a real hole, because the measurement window
never covers build and the payload can wait out the test.
