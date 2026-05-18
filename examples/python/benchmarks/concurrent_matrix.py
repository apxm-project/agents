#!/usr/bin/env python3
"""Concurrent multi-tenant matrix driver.

Spawns N copies of the chosen workload (``--graph``) in parallel
(differentiated by ``APXM_MATRIX_VARIANT``), times the batch wallclock,
and writes a CSV with one row per batch trial. Per-trial it also queries
``GET /v1/apxm/graphs/{id}`` for each tenant to capture pin telemetry.

The point of this harness is to exercise the four preconditions for
APXM's priority scheduler hint to do measurable work:
    1. ``--enable-prefix-caching`` ON                         (already met)
    2. KV utilization >= ``_APXM_PIN_ALLOW_USAGE`` (0.85)     (concurrent tenants generate this)
    3. ``--scheduling-policy priority``                       (already met)
    4. queue contention exists                                (this too)

Default workload: ``workloads/pin_demo.py``. The earlier default
``stress/prefix_fanout_concurrent.py`` is documented in
``KNOWN-ISSUES.md`` as reliably crashing vLLM engines on every model
APXM has tried; ``pin_demo`` is the known-working path that produced
INT-03's pin-latency-win claim.

Usage:
    python3 examples/python/benchmarks/concurrent_matrix.py \\
        --concurrency 4 --iterations 5 --opt-levels 0 2 \\
        --output .apxm/benchmarks/results/concurrent-matrix.csv
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

_TOOLS_SCRIPT_DIR = str(REPO_ROOT / "tools" / "scripts")
if _TOOLS_SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _TOOLS_SCRIPT_DIR)

from apxm_vllm_contract import ArmName, EnvVar  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent))
from util import prom_pull  # noqa: E402
from util import pre_registration  # noqa: E402

DEFAULT_GRAPH = REPO_ROOT / "examples" / "python" / "benchmarks" / "workloads" / "pin_demo.py"
DEFAULT_RESULTS_DIR = REPO_ROOT / ".apxm" / "benchmarks" / "results"
DEFAULT_OUTPUT = DEFAULT_RESULTS_DIR / "concurrent-matrix.csv"

APXM_DISABLE_HINTS_ENV = EnvVar.APXM_DISABLE_HINTS.value
DISABLE_HINTS_ENABLED = "1"
ARM_APXM_ON = ArmName.APXM_ON.value
ARM_FLAT_HTTP = ArmName.FLAT_HTTP.value

# Matrix cells are (prefix-cache state) × (arm). The previous shape
# (prefix × scheduling_policy) was retired when the APXM controller
# dropped FCFS support — the only supported policy is now `priority`,
# applied uniformly to all cells. The arm axis (APXM-on vs flat-HTTP)
# is the load-bearing comparison.
EXPECTED_MATRIX_CELLS = {
    "A": {"server_prefix_caching": False, "arm": ARM_FLAT_HTTP, "opt_level": 0},
    "B": {"server_prefix_caching": True, "arm": ARM_FLAT_HTTP, "opt_level": 0},
    "C": {"server_prefix_caching": False, "arm": ARM_APXM_ON, "opt_level": 2},
    "D": {"server_prefix_caching": True, "arm": ARM_APXM_ON, "opt_level": 2},
}

MATRIX_VARIANT_ENV = EnvVar.APXM_MATRIX_VARIANT.value
MATRIX_CELL_LABEL_ENV = "APXM_MATRIX_CELL_LABEL"
APXM_VLLM_CACHE_SALT_ENV = EnvVar.APXM_VLLM_CACHE_SALT.value


@dataclass
class TenantResult:
    cell_label: str
    server_scheduling_policy: str
    server_prefix_caching: bool
    max_num_seqs: int
    model: str
    service_name: str
    variant: int
    opt_level: int
    iteration: int
    returncode: int
    wall_ms: float
    execution_id: str
    pinned_blocks_peak: int
    cached_input_tokens: int
    llm_calls: int
    arm: str = ARM_APXM_ON


@dataclass
class BatchRow:
    timestamp: str
    cell_label: str
    server_scheduling_policy: str
    server_prefix_caching: bool
    max_num_seqs: int
    model: str
    service_name: str
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
    arm: str = ARM_APXM_ON
    prefix_cache_hit_rate: float | None = None
    prefix_cache_queries_delta: float = 0.0
    prefix_cache_hits_delta: float = 0.0


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--concurrency", type=int, default=4)
    p.add_argument("--iterations", type=int, default=5)
    p.add_argument("--opt-levels", type=int, nargs="+", default=[0, 2])
    p.add_argument("--graph", type=Path, default=DEFAULT_GRAPH)
    p.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    p.add_argument("--stagger-ms", type=int, default=200,
                   help="Submission stagger between tenants per batch.")
    p.add_argument("--apxm-endpoint", default=os.environ.get("APXM_ENDPOINT", ""),
                   help="Base URL for /v1/apxm/* (vLLM service endpoint without /v1). "
                        "Falls back to APXM_ENDPOINT env; no hardcoded port default.")
    p.add_argument("--target", default="latency")
    p.add_argument("--interleave-opt-levels", action="store_true", default=True)
    p.add_argument("--cell-label", default=os.environ.get(MATRIX_CELL_LABEL_ENV, ""),
                   help="Benchmark matrix cell label. Defaults to A/B/C/D inference from server controls and opt level.")
    p.add_argument("--server-scheduling-policy", default=os.environ.get("SCHEDULING_POLICY", "priority"))
    p.add_argument("--server-prefix-caching", action=argparse.BooleanOptionalAction,
                   default=_env_bool("ENABLE_PREFIX_CACHING", True))
    p.add_argument("--max-num-seqs", type=int, default=int(os.environ.get("MAX_NUM_SEQS", "64")))
    p.add_argument("--model", default=os.environ.get("MODEL_REF", ""))
    p.add_argument("--service-name", default=os.environ.get("APXM_VLLM_SERVICE_NAME", ""))
    p.add_argument("--matrix-report", type=Path,
                   help="Write a JSON coverage/lint report for the A-D benchmark matrix.")
    p.add_argument("--require-matrix", action="store_true",
                   help="Exit nonzero when --matrix-report finds missing or failed cells.")
    p.add_argument("--no-apxm-hints", action="store_true",
                   help="Run as the flat-HTTP control arm: suppress vllm_xargs.apxm injection "
                        "and /v1/apxm/graphs/register on the same vLLM build.")
    p.add_argument("--metrics-url", default=None,
                   help="vLLM Prometheus /metrics URL. Defaults to "
                        "<apxm_endpoint>/metrics. Set empty string to disable polling.")
    p.add_argument("--pre-registration", type=Path, default=None,
                   help="Pre-registration markdown file authored before the run. "
                        "Copied alongside the output CSV when provided.")
    p.add_argument("--require-pre-registration", action="store_true",
                   help="Refuse to start when --pre-registration is missing. "
                        "Use for runs whose evidence will be cited in docs/claims/.")
    return p.parse_args()


def _env_bool(name: str, default: bool) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    return raw.strip().lower() not in {"", "0", "false", "no", "off"}


def _cell_label(explicit: str, arm: str, prefix_caching: bool, opt_level: int) -> str:
    if explicit:
        return explicit
    for label, spec in EXPECTED_MATRIX_CELLS.items():
        if (
            spec["server_prefix_caching"] == prefix_caching
            and spec["arm"] == arm
            and spec["opt_level"] == opt_level
        ):
            return label
    return f"{arm}-prefix-{int(prefix_caching)}-o{opt_level}"


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
    cell_label: str,
    server_scheduling_policy: str,
    server_prefix_caching: bool,
    max_num_seqs: int,
    model: str,
    service_name: str,
    arm: str = ARM_APXM_ON,
) -> TenantResult:
    env = os.environ.copy()
    env[MATRIX_VARIANT_ENV] = str(variant)
    # Salt scoped per (arm, opt_level). Arm prevents the second arm of
    # a paired-arm A/B from hitting the first arm's cache (the prior
    # per-(iter, variant) scheme caused a ~99 % vs 0 % hit-rate inversion
    # — see docs/claims/mooncake-paired-arms-smoke.md). Opt_level
    # prevents O2 from inheriting O0's warm cache. Variant is NOT in
    # the salt: tenant differentiation comes from byte-distinct contexts
    # in the workload itself (each tenant uses its variant index to vary
    # the prompt body deterministically — see the workload's own docstring),
    # so all tenants in a cell legitimately share salt and the cache
    # reflects the workload's true prefix-sharing structure (which is
    # what we want for cohort-routing measurements).
    env[APXM_VLLM_CACHE_SALT_ENV] = f"matrix-arm-{arm}-opt-{opt_level}"
    import tempfile
    metrics_dir = Path(tempfile.mkdtemp(prefix=f"matrix-metrics-v{variant}-it{iteration}-"))
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
        cell_label=cell_label,
        server_scheduling_policy=server_scheduling_policy,
        server_prefix_caching=server_prefix_caching,
        max_num_seqs=max_num_seqs,
        model=model,
        service_name=service_name,
        variant=variant,
        opt_level=opt_level,
        iteration=iteration,
        returncode=proc.returncode,
        wall_ms=wall_ms,
        execution_id=execution_id,
        pinned_blocks_peak=pin_peak,
        cached_input_tokens=cached_input,
        llm_calls=llm_calls,
        arm=arm,
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
    cell_label_override: str,
    server_scheduling_policy: str,
    server_prefix_caching: bool,
    max_num_seqs: int,
    model: str,
    service_name: str,
    arm: str = ARM_APXM_ON,
    metrics_url: str | None = None,
) -> tuple[BatchRow, list[TenantResult]]:
    tenants: list[TenantResult] = []
    cell_label = _cell_label(
        cell_label_override,
        arm,
        server_prefix_caching,
        opt_level,
    )

    metrics_before: dict[str, float] = {}
    if metrics_url:
        try:
            metrics_before = prom_pull.snapshot(metrics_url)
        except (urllib.error.URLError, TimeoutError) as exc:
            print(f"[concurrent-matrix] warn: metrics snapshot before batch failed: {exc!r}", flush=True)

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
                cell_label=cell_label,
                server_scheduling_policy=server_scheduling_policy,
                server_prefix_caching=server_prefix_caching,
                max_num_seqs=max_num_seqs,
                model=model,
                service_name=service_name,
                arm=arm,
            ))
            if variant < concurrency - 1 and stagger_ms > 0:
                time.sleep(stagger_ms / 1000.0)
        for fut in as_completed(futures):
            tenants.append(fut.result())

    batch_wall_ms = (time.perf_counter() - batch_start) * 1000.0

    metrics_delta: dict[str, float] = {}
    if metrics_url and metrics_before:
        try:
            metrics_after = prom_pull.snapshot(metrics_url)
            metrics_delta = prom_pull.delta(metrics_before, metrics_after)
        except (urllib.error.URLError, TimeoutError) as exc:
            print(f"[concurrent-matrix] warn: metrics snapshot after batch failed: {exc!r}", flush=True)
    cell_hit_rate = prom_pull.hit_rate(metrics_delta) if metrics_delta else None
    queries_delta = metrics_delta.get(prom_pull.PREFIX_CACHE_QUERIES_TOTAL, 0.0)
    hits_delta = metrics_delta.get(prom_pull.PREFIX_CACHE_HITS_TOTAL, 0.0)

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
        cell_label=cell_label,
        server_scheduling_policy=server_scheduling_policy,
        server_prefix_caching=server_prefix_caching,
        max_num_seqs=max_num_seqs,
        model=model,
        service_name=service_name,
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
        arm=arm,
        prefix_cache_hit_rate=cell_hit_rate,
        prefix_cache_queries_delta=queries_delta,
        prefix_cache_hits_delta=hits_delta,
    )
    return row, tenants


def _write_matrix_report(
    rows: list[BatchRow],
    path: Path,
    *,
    pre_registration_path: str | None = None,
) -> bool:
    by_label: dict[str, list[BatchRow]] = {}
    for row in rows:
        by_label.setdefault(row.cell_label, []).append(row)

    cells = {}
    ok = True
    for label, spec in EXPECTED_MATRIX_CELLS.items():
        matching = [
            row for row in by_label.get(label, [])
            if row.opt_level == spec["opt_level"]
            and row.arm == spec["arm"]
            and row.server_prefix_caching == spec["server_prefix_caching"]
        ]
        failed_tenants = sum(row.failed_tenants for row in matching)
        cell_ok = bool(matching) and failed_tenants == 0
        if not cell_ok:
            ok = False
        cells[label] = {
            **spec,
            "present": bool(matching),
            "rows": len(matching),
            "failed_tenants": failed_tenants,
            "batch_wall_ms": [row.batch_wall_ms for row in matching],
            "pinned_blocks_peak_max": max(
                (row.pinned_blocks_peak_max for row in matching),
                default=0,
            ),
        }

    payload = {
        "matrix": "concurrent_matrix_2x2",
        "ok": ok,
        "cells": cells,
        "pre_registration_path": pre_registration_path,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    return ok


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
    args.server_scheduling_policy = args.server_scheduling_policy.strip().lower()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    if args.no_apxm_hints:
        os.environ[APXM_DISABLE_HINTS_ENV] = DISABLE_HINTS_ENABLED
    arm = ARM_FLAT_HTTP if args.no_apxm_hints else ARM_APXM_ON
    pre_reg_sidecar = pre_registration.enforce(
        pre_registration_arg=args.pre_registration,
        require=args.require_pre_registration,
        output_path=args.output.resolve(),
    )
    if args.metrics_url is None:
        metrics_url = f"{args.apxm_endpoint.rstrip('/')}/metrics"
    elif args.metrics_url == "":
        metrics_url = None
    else:
        metrics_url = args.metrics_url

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
        print(f"[concurrent-matrix] opt={opt} iter={it} concurrency={args.concurrency}", flush=True)
        row, tenants = _run_batch(
            graph=args.graph,
            opt_level=opt,
            iteration=it,
            concurrency=args.concurrency,
            stagger_ms=args.stagger_ms,
            target=args.target,
            apxm_endpoint=args.apxm_endpoint,
            cell_label_override=args.cell_label,
            server_scheduling_policy=args.server_scheduling_policy,
            server_prefix_caching=args.server_prefix_caching,
            max_num_seqs=args.max_num_seqs,
            model=args.model,
            service_name=args.service_name,
            arm=arm,
            metrics_url=metrics_url,
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
        "timestamp", "cell_label", "server_scheduling_policy",
        "server_prefix_caching", "max_num_seqs", "model", "service_name",
        "opt_level", "iteration", "concurrency",
        "batch_wall_ms", "max_tenant_wall_ms", "sum_tenant_wall_ms",
        "failed_tenants", "pinned_blocks_peak_max", "pinned_blocks_peak_sum",
        "cached_input_tokens_sum", "llm_calls_sum", "arm",
        "prefix_cache_hit_rate",
        "prefix_cache_queries_delta", "prefix_cache_hits_delta",
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

    print("\n=== Concurrent matrix batch summary ===")
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
    if args.matrix_report:
        matrix_ok = _write_matrix_report(
            rows, args.matrix_report, pre_registration_path=pre_reg_sidecar
        )
        print(f"Wrote Concurrent matrix matrix report to {args.matrix_report}")
        if args.require_matrix and not matrix_ok:
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
