#!/usr/bin/env python3
"""Phase-G concurrent multi-tenant driver.

Spawns N copies of ``stress/prefix_fanout_concurrent.py`` in parallel
(differentiated by ``APXM_PHASEG_VARIANT``), times the batch wallclock,
and writes a CSV with one row per batch trial. Per-trial it also queries
``GET /v1/apxm/graphs/{id}`` for each tenant to capture pin telemetry.

The point of this harness is to exercise the four preconditions for
APXM's Phase-C scheduler hint to do measurable work, identified in
``.apxm/evaluation/gptoss120b/runs/20260513T1616Z/phase-f-readiness.md``:
    1. ``--enable-prefix-caching`` ON                         (already met)
    2. KV utilization >= ``_APXM_PIN_ALLOW_USAGE`` (0.85)     (this is what concurrent tenants generate)
    3. ``--scheduling-policy priority``                       (already met)
    4. queue contention exists                                (this too)

Usage:
    python3 examples/python/benchmarks/phase_g_concurrent.py \\
        --concurrency 4 --iterations 5 --opt-levels 0 2 \\
        --output .apxm/benchmarks/results/phase-g-concurrent.csv
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
DEFAULT_GRAPH = REPO_ROOT / "examples" / "python" / "benchmarks" / "stress" / "prefix_fanout_concurrent.py"
DEFAULT_RESULTS_DIR = REPO_ROOT / ".apxm" / "benchmarks" / "results"
DEFAULT_OUTPUT = DEFAULT_RESULTS_DIR / "phase-g-concurrent.csv"


@dataclass
class TenantResult:
    variant: int
    opt_level: int
    iteration: int
    returncode: int
    wall_ms: float
    execution_id: str
    pinned_blocks_peak: int
    cached_input_tokens: int
    llm_calls: int


@dataclass
class BatchRow:
    timestamp: str
    opt_level: int
    iteration: int
    concurrency: int
    batch_wall_ms: float
    max_tenant_wall_ms: float
    sum_tenant_wall_ms: float
    failed_tenants: int
    pinned_blocks_peak_max: int
    pinned_blocks_peak_sum: int
    cached_input_tokens_sum: int
    llm_calls_sum: int


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--concurrency", type=int, default=4)
    p.add_argument("--iterations", type=int, default=5)
    p.add_argument("--opt-levels", type=int, nargs="+", default=[0, 2])
    p.add_argument("--graph", type=Path, default=DEFAULT_GRAPH)
    p.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    p.add_argument("--stagger-ms", type=int, default=200,
                   help="Submission stagger between tenants per batch.")
    p.add_argument("--apxm-endpoint", default="http://127.0.0.1:8916",
                   help="Base URL for /v1/apxm/* (vLLM service endpoint without /v1).")
    p.add_argument("--target", default="latency")
    p.add_argument("--interleave-opt-levels", action="store_true", default=True)
    return p.parse_args()


def _fetch_graph_status(endpoint: str, execution_id: str) -> dict:
    if not execution_id:
        return {}
    url = f"{endpoint.rstrip('/')}/v1/apxm/graphs/{execution_id}"
    try:
        req = urllib.request.Request(url, method="GET")
        with urllib.request.urlopen(req, timeout=5.0) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except (urllib.error.URLError, json.JSONDecodeError, TimeoutError):
        return {}


def _execute_tenant(
    *,
    graph: Path,
    opt_level: int,
    variant: int,
    iteration: int,
    target: str,
    apxm_endpoint: str,
) -> TenantResult:
    env = os.environ.copy()
    env["APXM_PHASEG_VARIANT"] = str(variant)
    # Salt by batch (iteration), not by execution: tenants in the same
    # batch share a salt so that within-tenant 8-way fan-out hits
    # auto-prefix-cache; across batches we rotate to keep trials
    # independent.
    env["APXM_VLLM_CACHE_SALT"] = f"phaseg-iter-{iteration}-variant-{variant}"
    import tempfile
    metrics_dir = Path(tempfile.mkdtemp(prefix=f"phaseg-metrics-v{variant}-it{iteration}-"))
    metrics_path = metrics_dir / "metrics.json"
    cmd = [
        "dekk", "apxm", "execute",
        str(graph),
        "-O", str(opt_level),
        "--target", target,
        "--json",
        "--emit-metrics", str(metrics_path),
        "--emit-metrics-level", "detailed",
        "--no-emit-session",
    ]
    start = time.perf_counter()
    proc = subprocess.run(
        cmd,
        env=env,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    wall_ms = (time.perf_counter() - start) * 1000.0

    execution_id = ""
    cached_input = 0
    llm_calls = 0
    pin_peak = 0
    if proc.stdout:
        try:
            obj = json.loads(proc.stdout)
            if isinstance(obj, dict):
                ex = obj.get("execution_id")
                if isinstance(ex, str):
                    execution_id = ex
                usage = obj.get("llm_usage") or {}
                llm_calls = int(usage.get("total_requests", 0) or 0)
                cached_input = int(usage.get("cached_input_tokens", 0) or 0)
        except json.JSONDecodeError:
            pass

    # Pull pin peak from the emitted metrics JSON. The detailed level
    # populates pinned_blocks_peak per backend graph; we max across them.
    if metrics_path.exists():
        try:
            metrics = json.loads(metrics_path.read_text())
            graphs = (
                metrics.get("backends", {}).get("graphs", [])
                if isinstance(metrics, dict) else []
            )
            if isinstance(graphs, list):
                for g in graphs:
                    if not isinstance(g, dict):
                        continue
                    p = int(g.get("pinned_blocks_peak", g.get("pinned_blocks", 0)) or 0)
                    if p > pin_peak:
                        pin_peak = p
            agg = (
                metrics.get("backends", {}).get("aggregate", {})
                if isinstance(metrics, dict) else {}
            )
            if isinstance(agg, dict):
                # cached_input_tokens shows up under aggregate in some
                # layouts; fall back to llm_usage from stdout otherwise.
                ci = int(agg.get("total_cached_input_tokens", 0) or 0)
                if ci > cached_input:
                    cached_input = ci
                lc = int(agg.get("total_requests", 0) or 0)
                if lc > llm_calls:
                    llm_calls = lc
        except (json.JSONDecodeError, OSError):
            pass

    # Best-effort cleanup of the temp metrics dir.
    try:
        if metrics_path.exists():
            metrics_path.unlink()
        metrics_dir.rmdir()
    except OSError:
        pass

    return TenantResult(
        variant=variant,
        opt_level=opt_level,
        iteration=iteration,
        returncode=proc.returncode,
        wall_ms=wall_ms,
        execution_id=execution_id,
        pinned_blocks_peak=pin_peak,
        cached_input_tokens=cached_input,
        llm_calls=llm_calls,
    )


def _run_batch(
    *,
    graph: Path,
    opt_level: int,
    iteration: int,
    concurrency: int,
    stagger_ms: int,
    target: str,
    apxm_endpoint: str,
) -> tuple[BatchRow, list[TenantResult]]:
    tenants: list[TenantResult] = []
    batch_start = time.perf_counter()

    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = []
        for variant in range(concurrency):
            futures.append(pool.submit(
                _execute_tenant,
                graph=graph,
                opt_level=opt_level,
                variant=variant,
                iteration=iteration,
                target=target,
                apxm_endpoint=apxm_endpoint,
            ))
            if variant < concurrency - 1 and stagger_ms > 0:
                time.sleep(stagger_ms / 1000.0)
        for fut in as_completed(futures):
            tenants.append(fut.result())

    batch_wall_ms = (time.perf_counter() - batch_start) * 1000.0
    failed = sum(1 for t in tenants if t.returncode != 0)
    if tenants:
        max_wall = max(t.wall_ms for t in tenants)
        sum_wall = sum(t.wall_ms for t in tenants)
        pin_peak_max = max(t.pinned_blocks_peak for t in tenants)
        pin_peak_sum = sum(t.pinned_blocks_peak for t in tenants)
        cached_sum = sum(t.cached_input_tokens for t in tenants)
        llm_calls_sum = sum(t.llm_calls for t in tenants)
    else:
        max_wall = sum_wall = 0.0
        pin_peak_max = pin_peak_sum = cached_sum = llm_calls_sum = 0

    row = BatchRow(
        timestamp=datetime.now(timezone.utc).isoformat(),
        opt_level=opt_level,
        iteration=iteration,
        concurrency=concurrency,
        batch_wall_ms=batch_wall_ms,
        max_tenant_wall_ms=max_wall,
        sum_tenant_wall_ms=sum_wall,
        failed_tenants=failed,
        pinned_blocks_peak_max=pin_peak_max,
        pinned_blocks_peak_sum=pin_peak_sum,
        cached_input_tokens_sum=cached_sum,
        llm_calls_sum=llm_calls_sum,
    )
    return row, tenants


def _bootstrap_speedup(
    o0_walls: list[float],
    o2_walls: list[float],
    *,
    n: int = 1000,
    seed: int = 1234,
) -> tuple[float, float, float]:
    import random
    rng = random.Random(seed)
    if not o0_walls or not o2_walls:
        return 0.0, 0.0, 0.0
    samples = []
    for _ in range(n):
        a = [rng.choice(o0_walls) for _ in o0_walls]
        b = [rng.choice(o2_walls) for _ in o2_walls]
        s = (sum(a) / len(a)) / (sum(b) / len(b))
        samples.append(s)
    samples.sort()
    mean_speedup = (sum(o0_walls) / len(o0_walls)) / (sum(o2_walls) / len(o2_walls))
    lo = samples[int(0.025 * n)]
    hi = samples[int(0.975 * n)]
    return mean_speedup, lo, hi


def main() -> int:
    args = _parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)

    iter_plan: list[tuple[int, int]] = []
    if args.interleave_opt_levels:
        for it in range(1, args.iterations + 1):
            for opt in args.opt_levels:
                iter_plan.append((opt, it))
    else:
        for opt in args.opt_levels:
            for it in range(1, args.iterations + 1):
                iter_plan.append((opt, it))

    rows: list[BatchRow] = []
    detail_rows: list[TenantResult] = []
    for (opt, it) in iter_plan:
        print(f"[phase-g] opt={opt} iter={it} concurrency={args.concurrency}", flush=True)
        row, tenants = _run_batch(
            graph=args.graph,
            opt_level=opt,
            iteration=it,
            concurrency=args.concurrency,
            stagger_ms=args.stagger_ms,
            target=args.target,
            apxm_endpoint=args.apxm_endpoint,
        )
        rows.append(row)
        detail_rows.extend(tenants)
        print(
            f"  batch_wall_ms={row.batch_wall_ms:.1f} "
            f"max_tenant_ms={row.max_tenant_wall_ms:.1f} "
            f"failed={row.failed_tenants} "
            f"pinned_peak_max={row.pinned_blocks_peak_max} "
            f"cached_sum={row.cached_input_tokens_sum}",
            flush=True,
        )

    fieldnames = list(asdict(rows[0]).keys()) if rows else [
        "timestamp", "opt_level", "iteration", "concurrency",
        "batch_wall_ms", "max_tenant_wall_ms", "sum_tenant_wall_ms",
        "failed_tenants", "pinned_blocks_peak_max", "pinned_blocks_peak_sum",
        "cached_input_tokens_sum", "llm_calls_sum",
    ]
    with args.output.open("w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=fieldnames)
        w.writeheader()
        for row in rows:
            w.writerow(asdict(row))

    detail_path = args.output.with_suffix(".tenants.csv")
    if detail_rows:
        det_fields = list(asdict(detail_rows[0]).keys())
        with detail_path.open("w", newline="") as fh:
            w = csv.DictWriter(fh, fieldnames=det_fields)
            w.writeheader()
            for r in detail_rows:
                w.writerow(asdict(r))

    print("\n=== Phase-G batch summary ===")
    by_opt: dict[int, list[float]] = {}
    by_opt_pin: dict[int, list[int]] = {}
    by_opt_failed: dict[int, int] = {}
    for r in rows:
        by_opt.setdefault(r.opt_level, []).append(r.batch_wall_ms)
        by_opt_pin.setdefault(r.opt_level, []).append(r.pinned_blocks_peak_max)
        by_opt_failed[r.opt_level] = by_opt_failed.get(r.opt_level, 0) + r.failed_tenants

    print(f"{'opt':>4} | {'n':>3} | {'mean batch ms':>14} | "
          f"{'p50':>9} | {'min':>9} | {'max':>9} | "
          f"{'pin_peak_max':>13} | {'failed':>6}")
    print("-" * 88)
    for opt in sorted(by_opt.keys()):
        walls = by_opt[opt]
        pins = by_opt_pin[opt]
        if not walls:
            continue
        m = sum(walls) / len(walls)
        med = statistics.median(walls)
        print(f"{opt:>4} | {len(walls):>3} | {m:>14.1f} | "
              f"{med:>9.1f} | {min(walls):>9.1f} | {max(walls):>9.1f} | "
              f"{max(pins):>13} | {by_opt_failed[opt]:>6}")

    if 0 in by_opt and 2 in by_opt:
        sp_mean, sp_lo, sp_hi = _bootstrap_speedup(by_opt[0], by_opt[2])
        rel = (1.0 - 1.0 / sp_mean) * 100.0
        abs_ms = (sum(by_opt[0]) / len(by_opt[0])) - (sum(by_opt[2]) / len(by_opt[2]))
        print()
        print(f"O0 vs O2 speedup (mean batch ms): {sp_mean:.3f}x  "
              f"95% CI [{sp_lo:.3f}, {sp_hi:.3f}]")
        print(f"Relative reduction: {rel:.2f}%   absolute: {abs_ms:.1f} ms ({abs_ms/1000.0:.2f} s)")
        print("Claim gates: rel>=5%? "
              f"{'PASS' if rel >= 5.0 else 'fail'}   "
              f"abs>=1.0s? {'PASS' if abs_ms >= 1000.0 else 'fail'}   "
              f"lower_bound>1.0? {'PASS' if sp_lo > 1.0 else 'fail'}")

    print(f"\nWrote {len(rows)} batch rows to {args.output}")
    if detail_rows:
        print(f"Wrote {len(detail_rows)} per-tenant rows to {detail_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
