# Pantheon GCP Test Harness

**What lives here:** the executable scripts every gate runbook depends on.

---

## Files

| File | Purpose |
|---|---|
| `runner-wrapper.sh` | Universal provision → run → capture → destroy wrapper. Every gate calls this. |
| `cost-tracker.py` | Generates `cost-report.json` from VM lifecycle + Spot price table + optional BigQuery billing export. |
| `kill-switch.sh` | Emergency all-region VM teardown. Dry-run by default; use `--confirm` to actually delete. |
| `finalize-evidence.py` | Renders manifest + summary + obsidian-note at run end from accumulated metrics. |
| `README.md` | This file. |

---

## Install on a VM

On the custom VM image (built in preflight Phase 6), install these under `/opt/pantheon-harness/`:

```bash
# During custom image bake
sudo mkdir -p /opt/pantheon-harness
sudo cp harness/*.py harness/*.sh /opt/pantheon-harness/
sudo chmod +x /opt/pantheon-harness/*.sh /opt/pantheon-harness/*.py

# Install Python dependencies
pip install pyyaml
```

The evidence-templates/ directory also gets copied to `/opt/pantheon-harness/templates/`.

---

## Local usage (on Mike's laptop)

For running a gate from the laptop side:

```bash
# Dry-run to see what a gate would provision
./runner-wrapper.sh --gate=2 --config=./configs/gate-2-dual-l4.env --dry-run

# Actually run it
./runner-wrapper.sh --gate=2 --config=./configs/gate-2-dual-l4.env

# Kill everything (dry-run)
./kill-switch.sh

# Actually kill everything
./kill-switch.sh --confirm

# Kill only a specific run's VMs
./kill-switch.sh --scope=run-id=gate2-dual-l4-... --confirm
```

---

## Gate config file format

Each gate has a config file in `configs/gate-N-*.env`:

```bash
# configs/gate-2-dual-l4.env
MACHINE_TYPE=g2-standard-24
ACCELERATOR="type=nvidia-l4,count=2"
MAX_RUN_DURATION_MIN=120
STARTUP_SCRIPT=./startup-scripts/gate-2-startup.sh
BOOT_DISK_SIZE=100
ATTACH_MODEL_DISK=true
EXPECTED_HYPOTHESES="H-2.1 H-2.2 H-2.3"
EXPECTED_COST_USD=0.90
IMAGE_FAMILY=pantheon-gpu
```

Each config is sourced by `runner-wrapper.sh` before provisioning.

---

## Kill-switch guardrails

The kill-switch is a safety device, not a routine operation. Rules:

1. **Always dry-run first.** Never `--confirm` on first invocation.
2. **`--nuclear` is for emergencies only.** Deletes ALL VMs in the project, not just Pantheon ones.
3. **Check orphan disks after.** The kill-switch deletes VMs, but if a disk was created with `auto-delete=no` (shouldn't happen in our config, but just in case), it persists.
4. **Billing alerts trigger a different path.** The hard-kill Cloud Function at the $50 threshold is independent of this script.

---

## Cost tracker modes

```bash
# Estimate from price table (immediate, default)
cost-tracker.py --run-id=... --output=... --mode=estimate

# Ground truth from BigQuery billing export (1-6 hr lag)
cost-tracker.py --run-id=... --output=... --mode=bigquery

# Both (estimate first, refine later)
cost-tracker.py --run-id=... --output=... --mode=both
```

Update `SPOT_PRICES_USD_PER_HOUR` in `cost-tracker.py` quarterly from GCP's pricing page.

---

## Shared responsibilities with runbooks

Runbooks in `../runbooks/` specify:
- What to test (hypotheses, measurements)
- Which model configs to launch
- When to self-destruct

This harness specifies:
- How to provision + destroy cleanly
- How to capture evidence
- How to generate cost reports + summaries
- How to kill things emergency-style

**Keep the separation.** Harness code is generic; runbook content is gate-specific.
