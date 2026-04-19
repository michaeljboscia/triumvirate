"""Pydantic models for evidence bundle schema (matches 20-EVIDENCE-BUNDLE-SPEC.md)."""

from __future__ import annotations

import datetime as dt
from typing import Any, Literal

from pydantic import BaseModel, Field


class TestMetrics(BaseModel):
    """Per-hypothesis metric block written to metrics/h-{N}-*.json."""

    schema_version: str = "1.0"
    test_id: str
    run_id: str
    hypothesis: str | None = None
    started_at: str
    ended_at: str
    duration_sec: float
    harness_mode: str
    endpoint: str | None = None
    model: str | None = None
    concurrency: int = 1
    input: dict[str, Any] = Field(default_factory=dict)
    output: dict[str, Any] = Field(default_factory=dict)
    metrics: dict[str, float | int | str] = Field(default_factory=dict)
    targets: dict[str, float | int | str] = Field(default_factory=dict)
    verdict: Literal["PASS", "FAIL", "INCONCLUSIVE"] = "INCONCLUSIVE"
    decision_rule_applied: str | None = None
    errors: list[dict[str, Any]] = Field(default_factory=list)


class ModelUsage(BaseModel):
    role: str
    model: str
    quantization: str | None = None


class Manifest(BaseModel):
    """Run manifest — becomes manifest.json in the evidence bundle."""

    schema_version: str = "1.0"
    run_id: str
    gate: int
    gate_name: str | None = None
    experimenter: str = "mike-boscia"
    triumvirate_version: str | None = None
    git_commit: str | None = None
    started_at: str
    ended_at: str | None = None
    duration_sec: float | None = None
    gcp_project: str | None = None
    gcp_region: str | None = None
    gcp_zone: str | None = None
    gcp_machine_types: list[str] = Field(default_factory=list)
    gcp_accelerators: list[str] = Field(default_factory=list)
    gcp_provisioning_model: str = "SPOT"
    models_used: list[ModelUsage] = Field(default_factory=list)
    hypotheses_tested: list[str] = Field(default_factory=list)
    verdicts: dict[str, str] = Field(default_factory=dict)
    overall_verdict: str = "INCONCLUSIVE"
    prior_runs_referenced: list[str] = Field(default_factory=list)
    decision_rules_applied: list[str] = Field(default_factory=list)
    total_cost_usd: float | None = None
    evidence_bundle_size_mb: float | None = None
    links: dict[str, str] = Field(default_factory=dict)

    @classmethod
    def new(cls, run_id: str, gate: int, **kwargs: Any) -> Manifest:
        return cls(
            run_id=run_id,
            gate=gate,
            started_at=dt.datetime.utcnow().isoformat() + "Z",
            **kwargs,
        )


class PromptRecord(BaseModel):
    """One dispatched prompt + its response, for evidence archival."""

    task_id: str
    prompt: str
    response: str = ""
    input_tokens: int = 0
    output_tokens: int = 0
    latency_ms: float = 0.0
    ttft_ms: float | None = None
    tool_calls_detected: int = 0
    json_validity: bool | None = None
    error: str | None = None
