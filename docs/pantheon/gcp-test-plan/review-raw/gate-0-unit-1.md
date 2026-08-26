# Raw peer output: gate-0-plumbing.md unit 1 (lines 1-111)

Header, Why this gate exists, What it does NOT test, Hypotheses, Pre-run checklist, Steps 1-2.

**Date:** 2026-08-26 · **Peers:** Codex (engineering), Gemini (strategic), DeepSeek (adversarial logic)

---

## CODEX (engineering angle)

**1. Hypotheses.**
- **H-0.1** has a real threshold: `<60 sec`, "All three healthy within 60 sec" (37-39). Falsifiable.
- **H-0.2** threshold is `5 canned tasks`, `5/5 tasks complete round-trip` (43-45). **Weak: "complete" can pass bad output unless structured result correctness is defined.**
- **H-0.3** has a threshold only by reference to `20-EVIDENCE-BUNDLE-SPEC.md` (51). Falsifiable if that spec is concrete, but this range does not require the newer `COMPLETE` sentinel written last.

**2. Step 1 local translation.**
- **Survives:** `RUN_ID` naming (72); a registry/image source concept (73) if containers are still pulled.
- **Pure GCP ceremony:** `PROJECT_ID`, `ZONE`, `gcloud compute instances list/create`, Spot VM, auto-delete, max-run-duration, VPC/subnet, service account, image family, `--no-address` (70-94).
- **Missing local equivalent:** validate `ssh lenovo`, Docker availability, GPU identity, compute capability 8.9, 12GB VRAM, 24 cores, 31GB RAM, disk space, expected repo/image availability, cleanup guard, and **a local run cap / kill mechanism.**

**3. Step 2 verifies almost nothing.**
- Only that `gcloud compute ssh ... docker ps` eventually succeeds (100-102).
- `docker ps` (107) checks daemon access, **not the required images, compose file, NATS, Triumvirate, mock vLLM, ports, or GPU.**
- `mkdir -p /tmp/evidence/$RUN_ID` (109) is **effectively non-failing** and proves no evidence contract.
- `gcloud auth configure-docker` (108) is registry setup, not verification.

**4. Pre-run checklist artifacts.**
- `10-PREFLIGHT.md` (57): EXISTS in repo.
- `pantheon-orchestrator-v1` VM image (58): DOES NOT EXIST, GCP-only.
- `pantheon-triumvirate:main`, `pantheon-test-harness:main`, `pantheon-nats:2.10`, `pantheon-vllm-cpu:v0.6.5` (59): **all DO NOT EXIST.**
- "No other Pantheon VMs live" (60): obsolete for the Lenovo.
- GCS evidence bucket writable (61): DOES NOT EXIST, obsolete if evidence is local-first.
- **The local Lenovo host is not mentioned in the checklist at all**, though it exists and has Docker, an RTX 4000 Ada, compute 8.9, 24 cores, 31GiB RAM.

**5. Scope boundary.** Mostly honest for "plumbing": excludes real inference, worker pool, protocol quirks, GPU scheduling (26-29). **But not sufficient for Track A's first runnable gate.** The gate claims end-to-end dispatch and evidence validity (3, 22), so it should not exclude output correctness for canned tasks or evidence completion semantics. **Otherwise it repeats the isolation-gate flaw: "tasks completed" instead of "outputs were correct."**

**6. Commands that fail today.** Lines 76-94 provision GCP resources instead of using `lenovo`, and depend on the nonexistent project, VPC, subnet, service account, and image family. Lines 100-104 fail because the GCP VM is never created. Line 108's registry auth is irrelevant locally. Line 107 succeeds on `lenovo` but the runbook does not target `lenovo`.

---

## GEMINI (strategic angle)

**1. What the gate is FOR now.** The original purpose (saving metered GPU dollars) is gone. **But the problem of isolating variables persists.** On a local box this gate is for **decoupling orchestration debugging from inference debugging.** Skip it and go straight to real models, and a timeout could be a NATS failure or a vLLM OOM. The gate ensures the plumbing works before adding physical GPU memory and model loading.

**2. What is missing: H-0.4, State Cleanup.** The three hypotheses cover startup (35), execution (41), and logging (47), which are right for the test itself. **Because the box is now persistent, you must prove the environment tears down cleanly. On a VM the machine disappears; on a local box, lingering containers or state will contaminate Gate 1.**

**3. Exit condition.** Currently just "does it work" (PASS). To prevent failure-to-advance and zombie infrastructure, **the exit condition must mandate a transition of state:** require successful execution of a teardown script that resets the local environment, and explicitly mandate proceeding to the next gate once teardown succeeds.

**4. What the self-destruct was FOR (84-85).** `--instance-termination-action=DELETE` plus `--max-run-duration=45m` was for **preventing budget bleed AND preventing zombie infrastructure.** The budget problem is gone. **The zombie infrastructure problem persists locally**: hung processes, port collisions, dangling Docker volumes, disk exhaustion. Replace with a local timeout wrapper (`timeout 45m ./run_gate_0.sh`) enforcing a teardown trap (`docker compose down -v`) **regardless of exit code.**

**5. The checklist's shape.** It relies on remote infrastructure and likely phantom documentation: `10-PREFLIGHT.md` (57), remote custom VM images (58), pushed registry images (59), GCS buckets (61). **A local checklist should verify local repository state, local Docker daemon health, and local directory structures, not cloud artifacts.**

---

## DEEPSEEK (adversarial logic angle)

Asked: the auto-delete was justified as cost control and dropped when the cost disappeared. What was its second job?

> The second job was **hygiene / ephemerality**: guaranteeing the environment was short-lived, fresh, unattended,
> unaccumulated, and eventually destroyed, not just cheap. It prevented stale state, secret/credential leakage,
> orphaned resources, unauthorized lingering access, and the drift that comes with long-lived test machines.
>
> **Unwritten secondary purposes accumulate because mechanisms solve whatever problem existed at the time, but only
> the headline reason gets documented.** Later, when the headline rationale disappears, people assume the mechanism is
> obsolete. **The unwritten job only becomes visible at removal time.**
>
> The general practice that catches this: **treat removal as a design decision, not a cleanup task.** Before deleting
> any safety mechanism, ask "What else is this doing?" and document every purpose it serves. Search history, ask the
> original authors, trace what would happen if it failed. **Do a "why does this exist?" review and write down all the
> answers before removing it.**

**Two-peer convergence, and it validates the owner's standing rule from first principles.** Gemini found the specific
second job by reading the runbook; DeepSeek derived the general case from the structure of the problem and named the
practice that catches it, **which is exactly the rule the owner imposed on this review after two bad deletions.**

DeepSeek adds a third purpose neither of us had: **credential and secret hygiene.** A long-lived local box
accumulates auth state that a disposable VM discards. That is a new finding for Track A, which now runs on a machine
that stays up.
