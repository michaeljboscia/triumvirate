# Pantheon Review Hit List

**Produced:** 2026-08-25
**Reviewers:** Claude (session), Codex (read files from disk), Gemini (read files from disk), DeepSeek (method-level only, see caveat)
**Scope:** the gate runbooks, the numbered plan documents, and the pantheon strategy corpus. Roughly 250KB that had received no real review before today.

**Headline:** the document that is now the entire product claim (`gate-6-airgap-sanity.md`) contradicts itself, and the corpus still contains an ACCEPTED document instructing a future reader to spend $160K on hardware the standing policy forbids.

**Caveat on DeepSeek:** it has no filesystem access through the Triumvirate bridge. It said so rather than fabricate citations. Its findings below are method-level and are marked as such. Its convergence with Codex matters precisely because the two reached the same conclusion from different directions: Codex from the file contents, DeepSeek from the logic alone.

---

## CRITICAL

### C-1. Gate 6 contradicts itself, and it is the product claim
`runbooks/gate-6-airgap-sanity.md`

The purpose section (lines 3, 45-48) states "ZERO outbound traffic" and "Any packet leaving the VM = fail." The decision rule (lines 278-280) passes the gate with `outbound packets <= 5`. Those cannot both be true. The document that gets shown to a prospect asserts a standard in its opening and applies a weaker one in its verdict.

*Caught by: Codex.*

### C-2. Gate 6 is not an air-gap test, it is a restricted-egress test
`runbooks/gate-6-airgap-sanity.md` lines 150-158, 258-259

The runbook explicitly permits Private Google Access to `199.36.153.8/30` and `199.36.153.4/30`. The honest description of what it proves is "no egress except Google private API endpoints." Calling that air-gap in front of a client security team is the kind of claim that ends an engagement.

Codex's recommended fix, which is correct: rename the GCP version to **cloud restricted-egress validation** and reserve "air-gap proof" for a physically disconnected box.

*Caught by: Codex.*

### C-3. Gate 0's self-destruct is unreachable
`runbooks/gate-0-plumbing.md` line 284

The `gcloud compute instances delete` command is placed *after* `exit` in the SSH session. As written it never runs. The VM survives until `--max-run-duration` or manual cleanup. This is layer 3 of the six advertised spend controls, and it is the third of those layers found to be non-functional (layers 4 and 6 have no implementation at all).

*Caught by: Codex.*

### C-4. An ACCEPTED document tells the reader to spend $160K
`HARDWARE_DECISION.md`

Still marked **Status: ACCEPTED**. States that the OPEX-first principle is a runway whose "first on-prem CapEx must clear the production floor," and charts a path to $160K of Tier 3 enterprise hardware. This directly contradicts `local-inference-buy-vs-rent.md` section 6 ("Never our capex").

Compounding it: `30-DECISION-RULES.md` Decision 10 sets an automatic trigger to convert OPEX to CAPEX once GCP spend exceeds $1000/month for two consecutive months. Gemini's description is apt: a loaded gun pointed at the runway.

*Caught by: Gemini.*

### C-5. The evidence bundle has no independence
`20-EVIDENCE-BUNDLE-SPEC.md` (method-level finding)

The system under test deploys itself, runs the gate, assembles the evidence, hashes it, and narrates it, with no root of trust outside the tested environment. A compromised or leaking system produces a byte-identical bundle to a clean one. That is the operational definition of evidence that discriminates nothing.

DeepSeek's framing: the defendant writing the police report. A hostile auditor is correct to refuse the conclusion.

*Caught by: DeepSeek (method-level). Needs confirmation against the actual file.*

---

## HIGH

### H-1. Gate 6's capture is blind to several exit paths
`runbooks/gate-6-airgap-sanity.md`

- tcpdump filter excludes only `10.128.0.0/20` and `127.0.0.0/8` (lines 127-130). It does not classify or exclude the metadata server, IPv6, link-local, multicast, DNS, or NTP.
- The firewall denies `0.0.0.0/0` but not `::/0` (lines 141-147). On a dual-stack VPC that is an open lane.
- `HF_HUB_OFFLINE=1` and `TRANSFORMERS_OFFLINE=1` are suggested only *after* a failure, not set during the actual run (lines 169-178, 285-291). Model registry callbacks are therefore not forced offline during the test.
- Build-time traffic is entirely outside the measurement window (lines 108-134). Anything fetched or phoning home during image creation, driver install, or fixture staging is invisible.

### H-2. Absence of evidence presented as evidence of absence
Method-level, converging with H-1.

Observing no egress during window W, over paths P, with instrumentation I, supports exactly that proposition. Concluding "no byte ever left on any path at any time" requires four closure conditions the runbook never argues: path coverage, window adequacy against plausible beacon intervals, instrumentation trustworthiness, and non-perturbation. A one-hour test cannot falsify a 24-hour beacon.

The fix is free and converts a story into data: **deflate the claim to match the method.** "During window W, on paths P, with this instrumentation, no egress was observed" is well-formed and falsifiable. Claim that, and the gate becomes sound.

*DeepSeek, converging with Codex's file-level findings.*

### H-3. Machine types and disks that do not exist or do not apply
- `g4-standard-32` is not a valid machine type. Used in `gate-6-airgap-sanity.md` at lines 5, 93, and hardcoded into the manifest at line 249. Also present in `harness/cost-tracker.py:45`. Nearest valid shape for one full RTX PRO 6000 is `g4-standard-48`.
- `--accelerator=type=nvidia-rtx-pro-6000,count=1` (line 94) is wrong for G4. The G series attaches GPUs automatically by machine type.
- `pd-ssd` (line 102) is likely invalid on G4, which does not support zonal or regional Persistent Disk and expects Hyperdisk or Titanium SSD.

### H-4. Gate 0 depends on roughly thirty things that were never built
`runbooks/gate-0-plumbing.md`

Codex enumerated the full list: the `pantheon-validation-v1` project, `pantheon-net` VPC and subnet, the validator service account, the `pantheon-orchestrator` image family, four container images, two harness run modes, four harness scripts under `/opt/pantheon-harness/`, the `gs://pantheon-evidence` bucket, and the Obsidian vault path. Additionally the compose file creation step is a literal placeholder (`# [paste content from above]`, lines 175-182) and is not executable.

Also note a naming mismatch: the checklist says `pantheon-orchestrator-v1` while the command uses `--image-family=pantheon-orchestrator`, and the checklist names `pantheon-vllm-cpu:v0.6.5` while the compose actually uses `pantheon-test-harness:main` as the mock vLLM.

### H-5. Preflight steps that cannot fail loudly
`10-PREFLIGHT.md` (method-level)

Grep for these classes: exit-code-only checks that print OK while doing nothing; a `grep` for the Cloud Function's source text *inside the very file the snippet is pasted into*, which exits 0 and "passes"; any `mkdir -p fixtures/` that silently creates the precondition it was meant to verify; and warnings that continue rather than fail. The only valid check for the kill-switch is functional: publish to the topic and assert the sink fires, then delete the function and assert preflight fails.

Over-broad IAM here is an epistemic defect, not just a security one. A process that can modify firewall rules or edit the evidence bundle taints everything it produces.

---

## MEDIUM

- Gate 6 firewall rules are torn down only at Step 8. Any earlier failure leaves `pantheon-airgap-deny-egress-$RUN_ID` and `pantheon-airgap-allow-pga-$RUN_ID` orphaned (lines 141, 264-271).
- Persistent billable artifacts are unaccounted: the `pantheon-models-v1` snapshot (called "transient" in the cost table at lines 303-307, which it is not), custom images, Artifact Registry storage, and three buckets.
- `30-DECISION-RULES.md` Decision 9 ties a Mac Studio purchase to WWDC 2026 expectations. The M5 Ultra was announced 2026-08-25 with the 512GB price unpublished.
- The V100 trap is absent from `graduated-gcp-validation-plan.md`. A reader optimizing for cheap OPEX would rent V100s and fail on sm_70.
- `--max-run-duration=45m` / `60m` suffix acceptance was not verified against current gcloud and should be checked live before execution.

---

## SALVAGE (do not delete these)

- **The TPS floor.** `HARDWARE_DECISION.md` establishes 15 tokens per second per stream under 4-way batched load as the production floor. This is substrate-agnostic: a rented config must clear it exactly as an owned one would. Extract it into the rent-first policy before archiving the file.
- **Decision rules 4, 5, 7, 8.** These gate plumbing, core thesis validation, and sovereign and production readiness. They validate software architecture and stability, which must pass regardless of who owns the metal. Only 1, 2, 3, 6, and 10 are pure CapEx triggers.

---

## CROSS-CUTTING PATTERNS

**1. The corpus is schizophrenic.** A permanent policy lives in one file while canonical ACCEPTED documents elsewhere present CapEx as the inevitable destination. A reader following the corpus today reaches the opposite conclusion from the policy. Banners bolted onto document headers do not fix this; the bodies have to change or the files have to be archived.

**2. Claims are inflated past what the method delivers.** Gate 6 says "zero egress" and measures "no unexpected egress observed from inside the guest during one run, except permitted Google API traffic." The evidence spec says "proof" and produces self-attested narrative. Six spend layers are advertised and three are now confirmed non-functional. The same failure repeats at every altitude: the document describes the intended end state in the present tense.

**3. Nothing was ever executed, so nothing forced reconciliation.** Every defect above survives because no run ever contradicted a claim. This is the root cause of the entire hit list, and it is an argument for running something cheap and real as early as possible rather than planning further.

**4. Verification steps that cannot fail are everywhere.** Grepping a file for a snippet pasted into that file. Creating the directory you meant to check for. Reporting a cost from a fallback machine type. Each one converts a broken precondition into a passing run.

---

## WHAT TO DO NEXT

1. **Archive `HARDWARE_DECISION.md` and `HARDWARE_DECISION_provenance.md`** into `archive/`, after extracting the TPS floor into `local-inference-buy-vs-rent.md`. Remove the ACCEPTED status first. This is the highest-value single action because that file currently instructs a $160K purchase.
2. **Delete decision rules 1, 2, 3, 6, 10** from `30-DECISION-RULES.md`. Keep 4, 5, 7, 8. Rule 10 is the automatic OPEX-to-CAPEX trigger and is the most dangerous line in the corpus.
3. **Rewrite gate-6.** Resolve the zero-versus-five contradiction, rename the GCP variant to cloud restricted-egress validation, deflate the claim to the observed-window form, cover IPv6 and the metadata server, set the offline environment variables during the run rather than after failure, and bring image build inside the measurement window.
4. **Fix gate-0's unreachable delete** (line 284) before any VM is ever created from that runbook.
5. **Move Track A to the local RTX 4000 Ada box.** Both Codex and this session's analysis agree independently: gate-0 gains nothing from GCP (it is CPU-only orchestration and every artifact it needs is unbuilt), and a physically disconnected box is categorically stronger air-gap evidence than a firewalled cloud VM, which always has a hypervisor, a metadata server, and PGA exceptions. Reserve GCP for the scale steps that actually need scale.
6. **Add external independence to the evidence bundle**: a capture point and attestation key that never touch the tested environment. Without at least one external component the bundle is structurally worthless no matter how good the artifacts look.
7. **Evaluate Cloudflare.** AI Gateway (identity-aware audit logs per prompt), R2 (US-only jurisdiction, no egress fees), Vectorize (regional pinning), Containers. If renting is permanent, the serverless edge provider was never assessed, and `docs/advisory/claude-deployment-options.md` maps only three doors where there are four.
