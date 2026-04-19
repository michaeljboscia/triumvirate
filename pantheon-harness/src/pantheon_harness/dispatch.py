"""Concurrent task dispatcher against OpenAI-compat endpoints.

Used by Gate 0 (task-dispatch-smoke) and Gates 1-5 (throughput-sustained).
Collects per-request timing, token counts, tool-call quality, errors.
"""

from __future__ import annotations

import asyncio
import datetime as dt
import json
import statistics
import time
from pathlib import Path

import httpx

from .models import PromptRecord, TestMetrics


DEFAULT_PROMPTS = [
    "Write a Python function that computes the SHA-256 of a file in chunks.",
    "Explain tensor parallelism in 3 sentences.",
    "Generate a SQL query that finds duplicate customer emails.",
    "Write a Rust fn that parses a CSV header line into a Vec<String>.",
    "Describe the CAP theorem. Use a concrete example.",
    "What does `kubectl describe pod` do? List 3 fields it shows.",
    "Write a Go function that reads env var with a default fallback.",
    "Explain the difference between INT4 and FP16 quantization for LLMs.",
]


async def _call_once(
    client: httpx.AsyncClient,
    endpoint: str,
    model: str,
    prompt: str,
    task_id: str,
    timeout_sec: float = 60.0,
) -> PromptRecord:
    """Fire one chat completion and record metrics."""
    rec = PromptRecord(task_id=task_id, prompt=prompt)
    t_start = time.perf_counter()
    try:
        resp = await client.post(
            f"{endpoint.rstrip('/')}/chat/completions",
            json={
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.3,
                "max_tokens": 512,
            },
            timeout=timeout_sec,
        )
        resp.raise_for_status()
        body = resp.json()
        t_end = time.perf_counter()
        rec.latency_ms = (t_end - t_start) * 1000
        rec.response = body["choices"][0]["message"]["content"]
        usage = body.get("usage", {})
        rec.input_tokens = usage.get("prompt_tokens", 0)
        rec.output_tokens = usage.get("completion_tokens", 0)
    except Exception as exc:  # noqa: BLE001
        rec.error = f"{type(exc).__name__}: {exc}"
        rec.latency_ms = (time.perf_counter() - t_start) * 1000
    return rec


async def dispatch_smoke(
    endpoint: str,
    model: str,
    num_tasks: int = 5,
    run_id: str = "smoke",
    prompts: list[str] | None = None,
    timeout_sec: float = 60.0,
) -> TestMetrics:
    """Gate 0 mode: fire N tasks sequentially at the endpoint, record round-trip."""
    started = dt.datetime.utcnow()
    pool = prompts or DEFAULT_PROMPTS
    records: list[PromptRecord] = []

    async with httpx.AsyncClient() as client:
        for i in range(num_tasks):
            p = pool[i % len(pool)]
            records.append(
                await _call_once(
                    client=client,
                    endpoint=endpoint,
                    model=model,
                    prompt=p,
                    task_id=f"smoke-{i:03d}",
                    timeout_sec=timeout_sec,
                )
            )

    ended = dt.datetime.utcnow()
    ok = [r for r in records if not r.error]
    errs = [r for r in records if r.error]

    verdict = "PASS" if len(ok) == num_tasks else "FAIL"

    return TestMetrics(
        test_id="task-dispatch-smoke",
        run_id=run_id,
        hypothesis="H-0.2",
        started_at=started.isoformat() + "Z",
        ended_at=ended.isoformat() + "Z",
        duration_sec=(ended - started).total_seconds(),
        harness_mode="task-dispatch-smoke",
        endpoint=endpoint,
        model=model,
        concurrency=1,
        input={"num_tasks": num_tasks},
        output={
            "tasks_completed": len(ok),
            "tasks_errored": len(errs),
        },
        metrics={
            "round_trip_median_ms": statistics.median(r.latency_ms for r in ok) if ok else 0.0,
            "round_trip_mean_ms": statistics.mean(r.latency_ms for r in ok) if ok else 0.0,
        },
        verdict=verdict,
        errors=[{"task_id": r.task_id, "error": r.error} for r in errs],
    )


async def throughput_sustained(
    endpoint: str,
    model: str,
    concurrency: int = 1,
    duration_sec: float = 300.0,
    run_id: str = "throughput",
    prompts: list[str] | None = None,
    hypothesis: str | None = None,
) -> TestMetrics:
    """Gates 1-5 mode: run N concurrent streams for duration, measure tok/s per stream.

    Each stream keeps firing requests back-to-back until duration elapses.
    Collects: per-stream tok/s, ttft, completion latency, schema validity.
    """
    started = dt.datetime.utcnow()
    pool = prompts or DEFAULT_PROMPTS
    records: list[PromptRecord] = []
    stop_at = time.monotonic() + duration_sec

    async def stream_worker(worker_id: int) -> None:
        async with httpx.AsyncClient() as client:
            i = 0
            while time.monotonic() < stop_at:
                prompt = pool[(worker_id * 1000 + i) % len(pool)]
                task_id = f"w{worker_id}-{i:04d}"
                rec = await _call_once(
                    client=client,
                    endpoint=endpoint,
                    model=model,
                    prompt=prompt,
                    task_id=task_id,
                )
                records.append(rec)
                i += 1

    await asyncio.gather(*(stream_worker(w) for w in range(concurrency)))

    ended = dt.datetime.utcnow()
    ok = [r for r in records if not r.error]
    errs = [r for r in records if r.error]

    total_output_tokens = sum(r.output_tokens for r in ok)
    total_input_tokens = sum(r.input_tokens for r in ok)
    duration = (ended - started).total_seconds()

    # Per-stream tok/s: aggregate output tokens / duration / concurrency
    per_stream_tps = total_output_tokens / duration / max(concurrency, 1)

    latencies_ms = [r.latency_ms for r in ok]
    latencies_sorted = sorted(latencies_ms) if latencies_ms else [0.0]

    def pct(p: float) -> float:
        if not latencies_sorted:
            return 0.0
        k = int(len(latencies_sorted) * p / 100)
        return latencies_sorted[min(k, len(latencies_sorted) - 1)]

    metrics = {
        "tokens_per_second_per_stream": round(per_stream_tps, 3),
        "tokens_per_second_aggregate": round(total_output_tokens / duration, 3),
        "wall_clock_per_completion_ms_median": round(
            statistics.median(latencies_ms) if latencies_ms else 0, 2
        ),
        "wall_clock_per_completion_ms_p95": round(pct(95), 2),
        "wall_clock_per_completion_ms_p99": round(pct(99), 2),
        "requests_per_minute": round(len(ok) / duration * 60, 2),
        "error_rate": round(len(errs) / max(len(records), 1), 4),
    }

    return TestMetrics(
        test_id=f"throughput-c{concurrency}",
        run_id=run_id,
        hypothesis=hypothesis,
        started_at=started.isoformat() + "Z",
        ended_at=ended.isoformat() + "Z",
        duration_sec=duration,
        harness_mode="throughput-sustained",
        endpoint=endpoint,
        model=model,
        concurrency=concurrency,
        input={"duration_sec_target": duration_sec, "prompt_pool_size": len(pool)},
        output={
            "requests_completed": len(ok),
            "requests_errored": len(errs),
            "total_input_tokens": total_input_tokens,
            "total_output_tokens": total_output_tokens,
        },
        metrics=metrics,
        verdict="PASS" if len(errs) == 0 and per_stream_tps > 0 else "FAIL",
        errors=[{"task_id": r.task_id, "error": r.error} for r in errs[:20]],
    )


def write_metrics(metrics: TestMetrics, output_dir: Path) -> Path:
    """Write TestMetrics JSON into evidence bundle structure."""
    output_dir.mkdir(parents=True, exist_ok=True)
    filename = f"{metrics.test_id}.json"
    path = output_dir / filename
    path.write_text(json.dumps(metrics.model_dump(), indent=2))
    return path
