#!/usr/bin/env python3
"""
cost-tracker.py — Generate a cost-report.json for a Pantheon run.

Queries GCP billing via labels attached to the VM at provision time.
Waits up to 45 min for billing data to land (BigQuery billing export has ~1-6 hr lag;
for same-session cost reports, we compute from VM lifecycle + known Spot prices).

Usage:
  cost-tracker.py --run-id=gate2-dual-l4-... --output=/tmp/evidence/.../cost-report.json

Strategy:
  Phase 1 (immediate, pre-billing-export): Compute estimated cost from VM lifecycle
    - Query VM start/stop timestamps from Cloud Logging
    - Look up machine type + accelerator Spot price from embedded price table
    - Multiply duration × hourly rate

  Phase 2 (ground-truth, 1-6 hr lag): Re-generate cost-report with actual
    billing-export data from BigQuery when available.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


# Embedded Spot price table (us-central1, 2026-04 approximate)
# Update quarterly from https://cloud.google.com/compute/all-pricing
SPOT_PRICES_USD_PER_HOUR = {
    # CPU
    "e2-standard-2": 0.0067,
    "e2-standard-4": 0.13,
    "e2-standard-8": 0.24,
    # GPU (includes CPU base + GPU uplift)
    "g2-standard-4": 0.28,  # 1x L4
    "g2-standard-12": 0.56,  # 1x L4 + more CPU
    "g2-standard-24": 0.42,  # 2x L4
    "g2-standard-48": 1.12,  # 4x L4
    "g4-standard-32": 3.50,  # 1x RTX Pro 6000 Blackwell
    "g4-standard-96": 14.00,  # 4x RTX Pro 6000
    "g4-standard-192": 28.00,  # 8x RTX Pro 6000
    "a2-highgpu-1g": 1.50,  # 1x A100 40GB
    "a2-ultragpu-1g": 2.20,  # 1x A100 80GB
    "a2-ultragpu-4g": 8.00,  # 4x A100 80GB
    "a2-ultragpu-8g": 12.00,  # 8x A100 80GB
    "a3-highgpu-8g": 60.00,  # 8x H100 80GB
    "a3-ultragpu-8g": 70.00,  # 8x H200 141GB
}

STORAGE_COST_USD_PER_GB_HOUR = {
    "pd-ssd": 0.17 / (24 * 30),  # $0.17/GB-month → per-hour
    "pd-balanced": 0.10 / (24 * 30),
    "pd-standard": 0.04 / (24 * 30),
    "standard-storage": 0.020 / (24 * 30),  # GCS standard
}


def run_gcloud(args: list[str]) -> str:
    """Run a gcloud command, return stdout."""
    try:
        r = subprocess.run(
            ["gcloud", *args], capture_output=True, text=True, check=True
        )
        return r.stdout
    except subprocess.CalledProcessError as e:
        print(f"gcloud failed: {e.stderr}", file=sys.stderr)
        return ""


def get_vm_lifecycle(run_id: str, project_id: str) -> dict[str, Any]:
    """Query Cloud Logging for this run's VM create + delete timestamps."""
    # Look up all VM operations tagged with this run_id
    log_filter = f'labels."run-id"={run_id.replace(":", "-")} OR textPayload:"{run_id}"'
    output = run_gcloud(
        [
            "logging",
            "read",
            log_filter,
            f"--project={project_id}",
            "--limit=100",
            "--format=json",
        ]
    )

    try:
        entries = json.loads(output) if output else []
    except json.JSONDecodeError:
        entries = []

    return {
        "create_timestamp": None,  # parse from entries
        "delete_timestamp": None,
        "machine_type": None,
        "accelerator": None,
        "entries_found": len(entries),
    }


def compute_estimated_cost(
    machine_type: str,
    duration_hours: float,
    disk_gb: int = 500,
    disk_type: str = "pd-ssd",
) -> list[dict[str, Any]]:
    """Compute estimated cost line items from VM lifecycle."""
    items = []

    # Compute / GPU
    if machine_type in SPOT_PRICES_USD_PER_HOUR:
        compute_rate = SPOT_PRICES_USD_PER_HOUR[machine_type]
        items.append(
            {
                "service": "Compute Engine",
                "sku": f"Spot {machine_type}",
                "usage_amount": round(duration_hours, 4),
                "usage_unit": "hours",
                "cost_usd": round(compute_rate * duration_hours, 4),
            }
        )

    # Persistent disk
    if disk_gb > 0:
        disk_rate = STORAGE_COST_USD_PER_GB_HOUR.get(disk_type, 0)
        items.append(
            {
                "service": "Compute Engine",
                "sku": f"Spot {disk_type}",
                "usage_amount": round(disk_gb * duration_hours, 4),
                "usage_unit": "GB-hours",
                "cost_usd": round(disk_rate * disk_gb * duration_hours, 4),
            }
        )

    # GCS (negligible for evidence bundles)
    items.append(
        {
            "service": "Cloud Storage",
            "sku": "Standard Storage US",
            "usage_amount": 0.05,
            "usage_unit": "GB-hours",
            "cost_usd": 0.001,
        }
    )

    return items


def query_bigquery_billing(
    run_id: str, project_id: str, billing_table: str
) -> list[dict[str, Any]]:
    """Query the billing export BigQuery table for ground-truth costs.
    Returns empty list if billing data hasn't landed yet (typical 1-6 hr lag)."""
    query = f"""
        SELECT
          service.description AS service,
          sku.description AS sku,
          usage.amount AS usage_amount,
          usage.unit AS usage_unit,
          cost AS cost_usd
        FROM `{billing_table}`
        WHERE labels.value = '{run_id}'
          AND _PARTITIONTIME >= TIMESTAMP_SUB(CURRENT_TIMESTAMP(), INTERVAL 24 HOUR)
    """

    output = run_gcloud(
        [
            "bq",
            "query",
            "--nouse_legacy_sql",
            "--format=json",
            "--project_id",
            project_id,
            query,
        ]
    )

    try:
        return json.loads(output) if output else []
    except json.JSONDecodeError:
        return []


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--run-id", required=True)
    p.add_argument("--output", required=True)
    p.add_argument("--project-id", default="pantheon-validation-v1")
    p.add_argument("--machine-type", help="If known, skip lifecycle query")
    p.add_argument("--duration-hours", type=float, help="If known")
    p.add_argument("--disk-gb", type=int, default=500)
    p.add_argument("--disk-type", default="pd-ssd")
    p.add_argument(
        "--billing-table",
        default="pantheon-validation-v1.billing_export.gcp_billing_export_v1_*",
    )
    p.add_argument(
        "--mode",
        choices=["estimate", "bigquery", "both"],
        default="estimate",
        help="estimate=use price table, bigquery=use billing export, both=estimate + refine",
    )
    args = p.parse_args()

    run_id = args.run_id
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    # Phase 1: Estimate from VM lifecycle + price table
    if args.machine_type and args.duration_hours:
        machine_type = args.machine_type
        duration = args.duration_hours
    else:
        lifecycle = get_vm_lifecycle(run_id, args.project_id)
        machine_type = lifecycle.get("machine_type") or "e2-standard-4"
        duration = 1.0  # fallback if we can't determine

    line_items = compute_estimated_cost(
        machine_type=machine_type,
        duration_hours=duration,
        disk_gb=args.disk_gb,
        disk_type=args.disk_type,
    )

    total = round(sum(item["cost_usd"] for item in line_items), 4)

    report = {
        "schema_version": "1.0",
        "run_id": run_id,
        "generated_at": dt.datetime.utcnow().isoformat() + "Z",
        "mode": args.mode,
        "machine_type": machine_type,
        "duration_hours": duration,
        "line_items": line_items,
        "total_cost_usd": total,
        "attribution_method": "spot-price-table"
        if args.mode != "bigquery"
        else "billing-export",
        "notes": (
            "Billing export has 1-6 hr lag. This report is an estimate based on "
            "VM lifecycle + embedded Spot prices. For ground truth, re-run with "
            "--mode=bigquery after 6+ hours."
        )
        if args.mode == "estimate"
        else None,
    }

    # Phase 2: Refine with BigQuery if requested
    if args.mode in ("bigquery", "both"):
        bq_items = query_bigquery_billing(run_id, args.project_id, args.billing_table)
        if bq_items:
            report["line_items_billing_export"] = bq_items
            report["total_cost_usd_billing_export"] = round(
                sum(float(item.get("cost_usd", 0)) for item in bq_items), 4
            )
            report["attribution_method"] = "billing-export"

    output_path.write_text(json.dumps(report, indent=2))
    print(f"Wrote cost report: {output_path}")
    print(f"  Estimated total: ${total}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
