"""pantheon-harness CLI — one entrypoint, mode flag selects behavior."""

from __future__ import annotations

import argparse
import asyncio
import sys
from pathlib import Path

from rich.console import Console

from . import __version__
from .dispatch import dispatch_smoke, throughput_sustained, write_metrics
from .mock_vllm import serve as serve_mock_vllm

console = Console()


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(
        prog="pantheon-harness",
        description=f"Pantheon GCP test harness v{__version__}",
    )
    p.add_argument(
        "--mode",
        required=True,
        choices=[
            "mock-vllm-server",
            "task-dispatch-smoke",
            "throughput-sustained",
            "multi-endpoint-concurrent",
        ],
        help="Which harness workload to execute",
    )
    p.add_argument(
        "--endpoint", default=None, help="OpenAI-compat endpoint (e.g. http://localhost:8000/v1)"
    )
    p.add_argument("--model", default=None, help="Model identifier to pass in requests")
    p.add_argument(
        "--triumvirate-url",
        default=None,
        help="Triumvirate daemon URL (for gates that go via orchestrator)",
    )
    p.add_argument("--concurrency", type=int, default=1)
    p.add_argument(
        "--duration", type=float, default=300.0, help="Duration (seconds) for sustained-throughput"
    )
    p.add_argument("--num-tasks", type=int, default=5, help="Task count for smoke tests")
    p.add_argument("--run-id", default=None)
    p.add_argument("--hypothesis", default=None)
    p.add_argument("--output-dir", type=Path, default=Path("/tmp/evidence"))
    p.add_argument("--host", default="0.0.0.0", help="Bind host for mock-vllm-server")
    p.add_argument("--port", type=int, default=8000, help="Bind port for mock-vllm-server")

    # multi-endpoint-concurrent mode args
    p.add_argument("--endpoint-a", default=None)
    p.add_argument("--endpoint-b", default=None)
    p.add_argument("--endpoint-c", default=None)
    p.add_argument("--endpoint-d", default=None)
    p.add_argument("--model-a", default=None)
    p.add_argument("--model-b", default=None)
    p.add_argument("--model-c", default=None)
    p.add_argument("--model-d", default=None)
    p.add_argument("--concurrency-a", type=int, default=1)
    p.add_argument("--concurrency-b", type=int, default=1)
    p.add_argument("--concurrency-c", type=int, default=0)
    p.add_argument("--concurrency-d", type=int, default=0)

    args = p.parse_args(argv)

    if args.mode == "mock-vllm-server":
        console.print(f"[bold green]Starting mock vLLM server on {args.host}:{args.port}[/]")
        serve_mock_vllm(host=args.host, port=args.port)
        return 0

    # All dispatch modes need endpoint + model
    if args.mode in ("task-dispatch-smoke", "throughput-sustained") and (
        not args.endpoint or not args.model
    ):
        console.print("[bold red]ERROR:[/] --endpoint and --model required for this mode")
        return 2

    output_dir: Path = args.output_dir
    run_id = args.run_id or f"{args.mode}-{Path(output_dir).name}"

    if args.mode == "task-dispatch-smoke":
        result = asyncio.run(
            dispatch_smoke(
                endpoint=args.endpoint,
                model=args.model,
                num_tasks=args.num_tasks,
                run_id=run_id,
            )
        )
        path = write_metrics(result, output_dir)
        console.print(f"Verdict: [bold]{result.verdict}[/]")
        console.print(f"  Completed: {result.output['tasks_completed']}/{args.num_tasks}")
        console.print(f"  Median round-trip: {result.metrics.get('round_trip_median_ms', 0):.1f}ms")
        console.print(f"  Wrote: {path}")
        return 0 if result.verdict == "PASS" else 1

    if args.mode == "throughput-sustained":
        result = asyncio.run(
            throughput_sustained(
                endpoint=args.endpoint,
                model=args.model,
                concurrency=args.concurrency,
                duration_sec=args.duration,
                run_id=run_id,
                hypothesis=args.hypothesis,
            )
        )
        path = write_metrics(result, output_dir)
        console.print(f"Verdict: [bold]{result.verdict}[/]")
        console.print(
            f"  Tok/s per stream: {result.metrics.get('tokens_per_second_per_stream', 0):.2f}"
        )
        console.print(
            f"  Tok/s aggregate: {result.metrics.get('tokens_per_second_aggregate', 0):.2f}"
        )
        console.print(f"  Requests completed: {result.output['requests_completed']}")
        console.print(
            f"  p95 latency: {result.metrics.get('wall_clock_per_completion_ms_p95', 0):.1f}ms"
        )
        console.print(f"  Wrote: {path}")
        return 0 if result.verdict == "PASS" else 1

    if args.mode == "multi-endpoint-concurrent":
        # Reuse throughput_sustained across multiple endpoints in parallel
        tasks = []
        for letter in "abcd":
            ep = getattr(args, f"endpoint_{letter}")
            m = getattr(args, f"model_{letter}")
            c = getattr(args, f"concurrency_{letter}")
            if ep and m and c > 0:
                tasks.append(
                    throughput_sustained(
                        endpoint=ep,
                        model=m,
                        concurrency=c,
                        duration_sec=args.duration,
                        run_id=f"{run_id}-{letter}",
                        hypothesis=args.hypothesis,
                    )
                )
        if not tasks:
            console.print("[bold red]ERROR:[/] no endpoint/model/concurrency triples provided")
            return 2
        results = asyncio.run(asyncio.gather(*tasks))
        for r in results:
            write_metrics(r, output_dir)
        all_pass = all(r.verdict == "PASS" for r in results)
        console.print(f"Overall verdict: [bold]{'PASS' if all_pass else 'FAIL'}[/]")
        return 0 if all_pass else 1

    return 2


if __name__ == "__main__":
    sys.exit(main())
