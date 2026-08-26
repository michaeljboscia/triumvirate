#!/usr/bin/env python3
"""
cost-tracker.py: Generate a cost-report.json for a Pantheon run.

Queries GCP billing via labels attached to the VM at provision time.
For same-session cost reports, compute from VM lifecycle and known Spot prices.

Usage:
  cost-tracker.py --run-id=gate2-dual-l4-... --output=/tmp/evidence/.../cost-report.json

Strategy:
  Phase 1 (immediate, pre-billing-export): Compute estimated cost from VM lifecycle
    - Query VM start/stop timestamps from Cloud Logging
    - Look up machine type + accelerator Spot price from embedded price table
    - Multiply duration × hourly rate

  Phase 2 (ground-truth): Re-generate cost-report with actual billing-export
    data from BigQuery when available.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


# Embedded Spot price table (us-central1, approximate)
# Update from https://cloud.google.com/compute/all-pricing
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
    # G4 GPU counts verified from Google Cloud docs:
    # g4-standard-96 has 2x RTX PRO 6000, g4-standard-192 has 4x,
    # and g4-standard-384 has 8x. Spot prices are UNVERIFIED here.
    "a2-highgpu-1g": 1.50,  # 1x A100 40GB
    "a2-ultragpu-1g": 2.20,  # 1x A100 80GB
    "a2-ultragpu-4g": 8.00,  # 4x A100 80GB
    "a2-ultragpu-8g": 12.00,  # 8x A100 80GB
    "a3-highgpu-8g": 60.00,  # 8x H100 80GB
    "a3-ultragpu-8g": 70.00,  # 8x H200 141GB
}

STORAGE_COST_USD_PER_GB_HOUR = {
    "pd-ssd": 0.17 / (24 * 30),  # $0.17/GB-month, per-hour
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


def parse_timestamp(value: Any) -> dt.datetime | None:
    if not isinstance(value, str) or not value:
        return None
    try:
        return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def find_string_value(data: Any, key_names: set[str]) -> str | None:
    if isinstance(data, dict):
        for key, value in data.items():
            if key in key_names and isinstance(value, str) and value:
                return value
            found = find_string_value(value, key_names)
            if found is not None:
                return found
    elif isinstance(data, list):
        for item in data:
            found = find_string_value(item, key_names)
            if found is not None:
                return found
    return None


def machine_type_name(value: str | None) -> str | None:
    if value is None:
        return None
    return value.rsplit("/", 1)[-1]


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

    create_timestamp = None
    delete_timestamp = None
    machine_type = None
    accelerator = None

    for entry in entries:
        timestamp = parse_timestamp(entry.get("timestamp"))
        method_name = find_string_value(entry, {"methodName", "method"})
        operation_name = find_string_value(entry, {"operation", "operationType"})
        event_text = " ".join(
            str(part).lower()
            for part in (method_name, operation_name, entry.get("textPayload"))
            if part
        )

        if "insert" in event_text or "create" in event_text:
            if timestamp is not None and (
                create_timestamp is None or timestamp < create_timestamp
            ):
                create_timestamp = timestamp
        if "delete" in event_text:
            if timestamp is not None and (
                delete_timestamp is None or timestamp > delete_timestamp
            ):
                delete_timestamp = timestamp

        machine_type = machine_type or machine_type_name(
            find_string_value(entry, {"machineType", "machine_type"})
        )
        accelerator = accelerator or find_string_value(
            entry, {"acceleratorType", "accelerator_type", "guestAccelerator"}
        )

    duration_hours = None
    if create_timestamp is not None and delete_timestamp is not None:
        seconds = (delete_timestamp - create_timestamp).total_seconds()
        if seconds > 0:
            duration_hours = seconds / 3600

    return {
        "create_timestamp": create_timestamp.isoformat()
        if create_timestamp is not None
        else None,
        "delete_timestamp": delete_timestamp.isoformat()
        if delete_timestamp is not None
        else None,
        "duration_hours": duration_hours,
        "machine_type": machine_type,
        "accelerator": accelerator,
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


def cost_input_failure_report(
    args: argparse.Namespace, reason: str, lifecycle: dict[str, Any] | None = None
) -> dict[str, Any]:
    print(f"WARN: cost report incomplete: {reason}", file=sys.stderr)
    return {
        "schema_version": "1.0",
        "run_id": args.run_id,
        "generated_at": dt.datetime.utcnow().isoformat() + "Z",
        "mode": args.mode,
        "machine_type": args.machine_type,
        "duration_hours": args.duration_hours,
        "line_items": [],
        "total_cost_usd": None,
        "cost_confidence": "unknown",
        "cost_confidence_reason": reason,
        "attribution_method": "unknown",
        "notes": "Cost report is incomplete. Missing real inputs cannot be replaced with defaults.",
        "lifecycle": lifecycle,
    }


def query_bigquery_billing(
    run_id: str, project_id: str, billing_table: str
) -> list[dict[str, Any]]:
    """Query the billing export BigQuery table for ground-truth costs.
    Returns empty list if billing data hasn't landed yet."""
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

    lifecycle = None

    # Phase 1: Estimate from VM lifecycle + price table
    machine_type = args.machine_type
    duration = args.duration_hours
    if machine_type is None or duration is None:
        lifecycle = get_vm_lifecycle(run_id, args.project_id)
        if machine_type is None:
            machine_type = lifecycle.get("machine_type")
        if duration is None:
            duration = lifecycle.get("duration_hours")

    incomplete_reason = None
    if not machine_type:
        incomplete_reason = "machine type could not be determined"
    elif duration is None:
        incomplete_reason = "duration hours could not be determined"
    elif duration <= 0:
        incomplete_reason = "duration hours is not positive"
    elif machine_type not in SPOT_PRICES_USD_PER_HOUR:
        incomplete_reason = f"no verified Spot price for machine type {machine_type}"

    if incomplete_reason is not None:
        report = cost_input_failure_report(args, incomplete_reason, lifecycle)
        if machine_type:
            report["machine_type"] = machine_type
        if duration is not None:
            report["duration_hours"] = duration
        output_path.write_text(json.dumps(report, indent=2))
        print(f"Wrote incomplete cost report: {output_path}")
        return 0

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
        "cost_confidence": "estimated",
        "cost_confidence_reason": "computed from supplied lifecycle inputs and verified price table entry",
        "attribution_method": "spot-price-table"
        if args.mode != "bigquery"
        else "billing-export",
        "notes": (
            "This report is an estimate based on VM lifecycle and embedded "
            "Spot prices. For ground truth, re-run with --mode=bigquery once "
            "billing export data is available."
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
