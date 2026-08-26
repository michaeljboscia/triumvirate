#!/usr/bin/env python3
"""
finalize-evidence.py: Render all required evidence bundle files at run end.

Runs on the VM as the last step before upload. Produces:
  - manifest.json       (finalized from start-of-run template + verdicts)
  - summary.md          (human-readable narrative from metrics)
  - cost-report.json    (via cost-tracker.py subprocess)
  - obsidian-note.md    (vault-ready markdown with YAML frontmatter)

Usage:
  finalize-evidence.py --run-id=... --gate=2 --evidence-dir=/tmp/evidence/...
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


def load_metrics(evidence_dir: Path) -> dict[str, Any]:
    """Read all metrics/h-*.json files into a dict keyed by test_id."""
    metrics = {}
    metrics_dir = evidence_dir / "metrics"
    if metrics_dir.exists():
        for mfile in metrics_dir.glob("h-*.json"):
            try:
                data = json.loads(mfile.read_text())
                metrics[data.get("test_id", mfile.stem)] = data
            except Exception as e:
                print(f"WARN: failed to parse {mfile}: {e}", file=sys.stderr)
    return metrics


def derive_verdicts(metrics: dict[str, Any]) -> dict[str, str]:
    """Extract verdict per hypothesis from metrics. If not set, INCONCLUSIVE."""
    verdicts = {}
    for test_id, data in metrics.items():
        hyp = data.get("hypothesis")
        if hyp:
            verdicts[hyp] = data.get("verdict", "INCONCLUSIVE")
    return verdicts


def overall_verdict(verdicts: dict[str, str]) -> str:
    if not verdicts:
        return "INCONCLUSIVE"
    values = list(verdicts.values())
    if all(v == "PASS" for v in values):
        return "PASS"
    if any(v == "FAIL" for v in values):
        return "FAIL"
    return "MIXED"


def finalize_manifest(
    run_id: str,
    gate: int,
    evidence_dir: Path,
    metrics: dict[str, Any],
    verdicts: dict[str, str],
) -> dict[str, Any]:
    """Update or create manifest.json with finalized state."""
    manifest_path = evidence_dir / "manifest.json"
    if manifest_path.exists():
        manifest = json.loads(manifest_path.read_text())
    else:
        manifest = {
            "schema_version": "1.0",
            "run_id": run_id,
            "gate": gate,
            "experimenter": "mike-boscia",
            "started_at": dt.datetime.utcnow().isoformat() + "Z",
        }

    manifest["ended_at"] = dt.datetime.utcnow().isoformat() + "Z"
    manifest["hypotheses_tested"] = sorted(verdicts.keys())
    manifest["verdicts"] = verdicts
    manifest["overall_verdict"] = overall_verdict(verdicts)
    manifest["metrics_files"] = [
        str(p.relative_to(evidence_dir))
        for p in (evidence_dir / "metrics").glob("h-*.json")
    ]

    manifest_path.write_text(json.dumps(manifest, indent=2))
    return manifest


def render_summary(
    manifest: dict[str, Any],
    metrics: dict[str, Any],
) -> str:
    """Render summary.md from manifest + metrics."""
    lines = []
    lines.append(f"# Run {manifest['run_id']}: Gate {manifest['gate']}")
    lines.append("")
    lines.append(f"**Date:** {manifest.get('started_at', 'unknown')}")
    lines.append(f"**Duration:** {manifest.get('ended_at', '')} (end)")
    lines.append(f"**Overall verdict:** {manifest.get('overall_verdict', 'UNKNOWN')}")
    lines.append("")
    lines.append("## Setup")
    lines.append(f"- Machine: {manifest.get('gcp_machine_types', ['unknown'])}")
    lines.append(f"- Accelerators: {manifest.get('gcp_accelerators', ['none'])}")
    lines.append("")
    lines.append("## Hypotheses tested")
    lines.append("")
    for hyp_id, verdict in sorted(manifest.get("verdicts", {}).items()):
        lines.append(f"### {hyp_id}")
        lines.append(f"**Verdict:** {verdict}")

        for test_id, data in metrics.items():
            if data.get("hypothesis") != hyp_id:
                continue
            m = data.get("metrics", {})
            if m:
                lines.append("")
                lines.append("| Metric | Value |")
                lines.append("|---|---|")
                for key, val in m.items():
                    if isinstance(val, (int, float)):
                        lines.append(f"| {key} | {val} |")
                lines.append("")

    lines.append("## Links")
    lines.append(
        f"- Evidence bundle: `gs://pantheon-evidence/gate-{manifest['gate']}/{manifest['run_id']}/`"
    )
    lines.append("")

    return "\n".join(lines)


def render_obsidian_note(
    manifest: dict[str, Any],
    metrics: dict[str, Any],
) -> str:
    """Render obsidian-note.md with YAML frontmatter + body."""
    verdicts = manifest.get("verdicts", {})
    models = manifest.get("models_used", [])

    frontmatter = {
        "type": "pantheon-run",
        "run_id": manifest["run_id"],
        "date": manifest.get("started_at", "")[:10],
        "gate": manifest["gate"],
        "gcp_machine": manifest.get("gcp_machine_types", ["unknown"])[0],
        "cost_usd": manifest.get("total_cost_usd", "unknown"),
        "hypotheses_tested": sorted(verdicts.keys()),
        "verdicts": verdicts,
        "overall_verdict": manifest.get("overall_verdict", "UNKNOWN"),
        "significance": 3,
        "tags": [
            "pantheon-run",
            f"gate-{manifest['gate']}",
            manifest.get("overall_verdict", "unknown").lower(),
        ],
    }

    import yaml

    fm_yaml = yaml.safe_dump(frontmatter, default_flow_style=False, sort_keys=False)

    body = [
        f"# Run {manifest['run_id']}: Gate {manifest['gate']}",
        "",
        f"**Verdict:** {manifest.get('overall_verdict', 'UNKNOWN')}",
        "",
        "## Summary",
        f"Gate {manifest['gate']} execution on {frontmatter['gcp_machine']}.",
        "",
        "## Hypotheses",
    ]

    for hyp_id, verdict in sorted(verdicts.items()):
        body.append(f"### [[{hyp_id}]]")
        body.append(f"**Result:** {verdict}")
        body.append("")

    body.extend(
        [
            "## Mike's notes",
            "<!-- Add qualitative observations after reviewing the bundle -->",
            "",
            "## Links",
            f"- Evidence bundle: `gs://pantheon-evidence/gate-{manifest['gate']}/{manifest['run_id']}/`",
        ]
    )

    return f"---\n{fm_yaml}---\n\n" + "\n".join(body)


def generate_cost_report(
    run_id: str, evidence_dir: Path, machine_type: str | None, duration_hours: float | None
):
    """Subprocess to cost-tracker.py."""
    cost_script = Path("/opt/pantheon-harness/cost-tracker.py")
    if not cost_script.exists():
        # Fallback location
        cost_script = Path(__file__).parent / "cost-tracker.py"

    output = evidence_dir / "cost-report.json"
    cmd = [
        "python3",
        str(cost_script),
        f"--run-id={run_id}",
        f"--output={output}",
    ]
    if machine_type is not None:
        cmd.append(f"--machine-type={machine_type}")
    if duration_hours is not None:
        cmd.append(f"--duration-hours={duration_hours}")

    try:
        subprocess.run(cmd, check=True)
    except subprocess.CalledProcessError as e:
        print(f"Cost report generation failed: {e}", file=sys.stderr)


def parse_timestamp(value: Any) -> dt.datetime | None:
    """Parse manifest timestamps emitted with a trailing Z."""
    if not isinstance(value, str) or not value:
        return None
    try:
        return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return None


def duration_hours_from_manifest(manifest: dict[str, Any]) -> float | None:
    started_at = parse_timestamp(manifest.get("started_at"))
    ended_at = parse_timestamp(manifest.get("ended_at"))
    if started_at is None or ended_at is None:
        return None

    seconds = (ended_at - started_at).total_seconds()
    if seconds <= 0:
        return None
    return seconds / 3600


def machine_type_from_manifest(manifest: dict[str, Any]) -> str | None:
    machine_types = manifest.get("gcp_machine_types")
    if isinstance(machine_types, list) and machine_types:
        machine_type = machine_types[0]
        if isinstance(machine_type, str) and machine_type:
            return machine_type
    return None


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--run-id", required=True)
    p.add_argument("--gate", type=int, required=True)
    p.add_argument("--evidence-dir", required=True, type=Path)
    args = p.parse_args()

    evidence_dir = args.evidence_dir
    evidence_dir.mkdir(parents=True, exist_ok=True)

    metrics = load_metrics(evidence_dir)
    verdicts = derive_verdicts(metrics)
    manifest = finalize_manifest(
        args.run_id, args.gate, evidence_dir, metrics, verdicts
    )

    summary_md = render_summary(manifest, metrics)
    (evidence_dir / "summary.md").write_text(summary_md)

    obsidian_md = render_obsidian_note(manifest, metrics)
    (evidence_dir / "obsidian-note.md").write_text(obsidian_md)

    # Cost report is best-effort, but missing inputs must be visible.
    machine_type = machine_type_from_manifest(manifest)
    duration_hours = duration_hours_from_manifest(manifest)
    if machine_type is None:
        print(
            "WARN: cost report incomplete: manifest has no gcp_machine_types value",
            file=sys.stderr,
        )
    if duration_hours is None:
        print(
            "WARN: cost report incomplete: cannot compute duration from started_at and ended_at",
            file=sys.stderr,
        )
    generate_cost_report(args.run_id, evidence_dir, machine_type, duration_hours)

    print(f"Finalized evidence bundle at {evidence_dir}")
    print(f"  Overall verdict: {manifest['overall_verdict']}")
    print(f"  Verdicts by hypothesis: {verdicts}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
