# Gate 0: Plumbing

**Status:** rewritten 2026-08-26 against a three-unit, three-peer review of the 2026-04-18 original
**Target:** the local box, host alias `lenovo` (`newlenovo`), not a cloud VM
**Review record:** `../REVIEW-PROGRESS.md` and `../review-raw/gate-0-unit-*.md`
**Original:** git at `8b2fae7:docs/pantheon/gcp-test-plan/runbooks/gate-0-plumbing.md`

> **This is the first thing that will actually run.** Nothing in this corpus has ever been executed, and every finding
> in the review record exists because reading was the only check available. That ends here.

---

## 1. What this gate is for

**Isolating variables.** If you go straight to real models, a timeout is ambiguous: it could be a NATS failure, a
container that never became ready, a config error, or a vLLM out-of-memory. This gate makes the plumbing known-good
first, so that when the next stage breaks you already know where it did not break.

That purpose is substrate-independent, which is why the gate survives the move from a metered cloud VM to a machine
we own, even though almost none of its original steps do. The original rationale ("prove orchestration works before
spending GPU dollars") is gone. This one replaces it.

## 2. What a PASS licenses you to believe, and what it does not

**Licensed:** Docker networking, NATS messaging, and Triumvirate configuration communicate. Tasks route through the
pipeline and come back correlated. The environment tears down cleanly.

**Not licensed:** anything about GPU allocation, CUDA drivers, model loading, real vLLM stability, request schema
differences, streaming, memory pressure under real inference, or latency.

**Inference is deliberately mocked, and mocking is the right call here** because it is what isolates the variable.
But be clear-eyed that it hides exactly the components most likely to fail next. A green Gate 0 is a statement about
plumbing and nothing else.

## 3. Hypotheses

Each is a pre-registered prediction with a threshold, committed before the run.

### H-0.1: the orchestration layer starts cleanly

**Prediction:** all three services reach *functional* readiness within 60 seconds of `compose up`.

**Threshold:** NATS accepts a connection, the mock inference endpoint answers, and Triumvirate reports it has
connected to NATS.

> **Readiness is not an HTTP 200.** The original defined healthy as a successful curl against a host port, which a
> service can satisfy while being unable to reach NATS or dispatch anything. Each check must exercise the dependency
> the service actually needs.

### H-0.2: tasks route end to end with their envelope intact

**Prediction:** 5 canned tasks traverse the full path and return correlated.

**Threshold:** for every task, the correlation ID that entered is the correlation ID that returns, each task is
observed at each stage it should pass through, and the routing envelope is uncorrupted.

> **This is the finding that changed the test.** The original asserted `tasks_completed: 5, tasks_errored: 0`, which
> an empty or malformed result satisfies. Two reviewers then disagreed about the fix, one arguing for output
> correctness and one arguing a plumbing test must never assert output semantics. Both were right about different
> things, and the boundary is the envelope:
>
> - **Assert:** task `#3` entered the mocked stage carrying `req-3` and its response returned on the right topic with
>   the same correlation ID, **regardless of whether the body is `{"ok":true}` or `"malformed"`**.
> - **Do not assert:** that the mock returned a semantically correct answer. That tests the mock's hardcoded
>   behavior, not the pipeline.
>
> **Do not assert latency either.** The original recorded a round-trip median in milliseconds, which against a mock
> measures the mock and whatever else the machine was doing. Latency belongs in the sizing sweep, against real
> inference.

### H-0.3: the evidence bundle is emitted correctly

**Prediction:** a bundle is produced that satisfies `../20-EVIDENCE-BUNDLE-SPEC.md`.

**Threshold:** every required object is present with its content hash recorded in the manifest, and the `COMPLETE`
sentinel is the **last** object written.

### H-0.4: the environment tears down cleanly

**Prediction:** after teardown, the machine is in the state it was in before the run.

**Threshold:** no Gate 0 containers running, the compose network removed, test-scoped volumes removed, all four bound
ports free, and no temp config or staged evidence left behind.

> **H-0.4 is new, and it exists because the machine is now persistent.** On a disposable VM this was free: the machine
> vanished. The VM auto-delete was documented as cost control, but it was also enforcing ephemerality, and only the
> cost half died with the move. Lingering containers, held ports, dangling volumes, and stale temp files contaminate
> the next run. A third purpose the peers surfaced: **credential hygiene**, since a long-lived box accumulates auth
> state a disposable one discards.

---

## 4. Pre-run checks

Every item is a command whose failure is visible. The original checklist listed eight items of which five referenced
artifacts that do not exist, and omitted the one machine that does.

```bash
set -euo pipefail

# The machine
ssh lenovo 'hostname && nproc && free -g | head -2'
ssh lenovo 'docker info >/dev/null && echo "docker OK"'
ssh lenovo 'df -h /var/lib/docker | tail -1'          # disk headroom for images and volumes

# Ports must be FREE before we start. This is the check whose absence causes
# the most confusing failure mode on a machine that stays up.
ssh lenovo 'for p in 4222 8222 8000 7788; do
  if lsof -i :$p >/dev/null 2>&1; then echo "PORT $p IN USE"; exit 1; fi
done; echo "ports OK"'

# No leftovers from a previous run
ssh lenovo 'docker ps -a --filter label=pantheon.gate=0 --format "{{.Names}}"'   # must be empty
```

**NOT BUILT, and required before this gate can run:**

- The container images. `pantheon-triumvirate`, `pantheon-nats`, and a mock inference image must be built locally and
  **pinned by digest**. Note `vllm/vllm-openai:v0.6.5-cpu` **does not exist upstream** (404); the CPU images live in a
  separate repository.
- The harness scripts. The original called them at `/opt/pantheon-harness/`, an absolute path native to a GCP custom
  image that was never built. **On the local box that directory does not exist and every call fails immediately with
  file-not-found. This is the single most likely first-run failure.** Decide the local path and use it consistently.
- The canonical task fixtures. See the fixtures section of `../10-PREFLIGHT.md`. **No gate may be committed until its
  fixtures exist and validate**, so this gate is not runnable until they do.

---

## 5. Running it

### The wrapper, which is not optional

```bash
timeout 45m ./run-gate-0.sh
```

The 45-minute bound survives the move to local, **for a different reason than it originally existed.** On GCP it
capped billing. Here it stops a hung process from indefinitely squatting ports 4222, 8222, 8000, and 7788 and
blocking every future run.

`run-gate-0.sh` must install a teardown trap **before** starting anything:

```bash
set -euo pipefail
trap teardown EXIT INT TERM
```

**The trap is the point.** Teardown must run on success, on failure, on timeout, and on interrupt. The original put
cleanup at the end of a linear script, and even that never executed because it sat after an `exit`.

### Step 1: start the stack

Compose file, config, and a **project name** so resources are namespaced and can be found later:

```bash
docker compose -p gate0-${RUN_ID} -f compose.gate-0.yml up -d
```

Label every service `pantheon.gate=0` so teardown and the pre-run leftover check can find them by label rather than
by guessing names.

**Health checks must exercise real dependencies**, and must not assume a tool exists inside an image. The original's
checks called `wget` in the NATS image and `curl` in the harness images, either of which can fail for reasons
unrelated to health.

**Wait deterministically, do not sleep.** The original used a blind `sleep 15`, which is both slower than necessary
when things are fine and silently insufficient when they are not. Poll for readiness with a bounded deadline and fail
loudly at the deadline.

### Step 2: H-0.1, functional readiness

Assert the three conditions from H-0.1. Record which one failed if any did, not just that the set failed.

### Step 3: H-0.2, envelope routing

Dispatch the 5 canned tasks with distinct correlation IDs. For each, assert the ID round-tripped and the task was
observed at each expected stage. **Bound this step with its own timeout**; the original had none, so a blocked
harness hangs the whole run until the outer 45 minutes expire.

### Step 4: H-0.3, evidence

Write the bundle per `../20-EVIDENCE-BUNDLE-SPEC.md`: objects first, content hashes in the manifest, **`COMPLETE`
sentinel last.**

> **Nothing may write to the bundle after this step.** The original named a step "evidence bundle emission" and then
> wrote logs, a cost report, and a note into the same bundle afterwards. A step name is a claim about an invariant.
> Only the step that leaves the artifact in its terminal state may be called emission. Anything a later step needs to
> record goes in a separate object written *before* the sentinel, or in a sidecar outside the bundle.

### Step 5: teardown, H-0.4

Ordered, and running unconditionally from the trap:

1. **Capture logs first**, before anything is destroyed.
2. `docker compose -p gate0-${RUN_ID} down -v --remove-orphans`
3. Archive staged evidence to permanent local storage.
4. Remove temp compose and config files, and the staging directory.
5. Verify the four ports are free again and no labelled containers remain. **Assert this, do not assume it.** That
   assertion is H-0.4.

**Do not** remove shared Docker resources, images other runs depend on, or long-lived local credentials. Scope every
removal to this run via the project name and the label. A blunt cleanup that takes out a neighbour's state is worse
than the contamination it was preventing.

---

## 6. Resource accounting

The original reported a dollar figure for a Spot VM. On a machine we own that number is meaningless.

**What the cost section was for:** preventing budget drain. **That problem does not exist locally. A constraint
problem still does**, so track the resources that are actually scarce here:

- **Disk consumed** by images, volumes, and archived evidence, with a stated ceiling.
- **Wall-clock duration** of the run, since operator waiting time and port locking are the real costs.

If anything in a future variant uploads to cloud storage, record `cost_status: pending_billing_export` rather than a
number. Billing export is not real-time, so an authoritative figure does not exist when the bundle seals.

## 7. On a PASS

**Passing this gate must move you forward, not license you to stop.**

The original said a PASS leads to the next gate, which is a passive statement of fact. Under Rule A of
`../30-DECISION-RULES.md`, further local iteration after a PASS requires a written reason, logged and peer-reviewed on
the same terms as a rule amendment. **The default is to advance.**

So this section ends with the literal command for the next stage rather than a description of it. An unlogged,
unreviewed reason to keep tinkering does not count.

## 8. On a FAIL

Record which hypothesis failed and at which stage. A failure here is the cheapest possible failure and the entire
reason the gate exists: it is far better to find a broken message path with a mock than to find it while also
debugging CUDA.

**Do not** proceed to the next stage on a partial pass. H-0.4 in particular must pass, because a contaminated
environment makes the next run's result untrustworthy in a way that is very hard to notice.
