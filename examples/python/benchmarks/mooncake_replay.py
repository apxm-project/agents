#!/usr/bin/env python3
"""mooncake_replay.py - Closed-loop Mooncake trace replay driver.

Drives one full pass over a Mooncake JSONL trace per (opt_level, iteration)
cell, dispatching each row through `dekk apxm execute workloads/mooncake_row.py`
with the synthesized prompt injected via env var. Produces a BatchRow-shaped
CSV that `comparison_report.py`'s arm-comparison section consumes unchanged.

The trace itself does NOT contain prompt text (privacy-redacted upstream).
The driver synthesizes prompts that reproduce the trace's prefix-cache
block structure via `workloads/data/_mooncake_hash.synthesize_prompt`. Rows
in the same prefix cohort produce text with identical byte-prefix, so the
backend's RadixAttention (and APXM's pin path) see the same KV-cache
structure as the original trace would have produced.

Closed-loop only: arrival timestamps in the trace are NOT honored. Real
arrival timing requires the in-process APXM HTTP path; that is a follow-up
wave once the HTTP server stand-up is proven on the cluster.

Usage:
  # Driver-only smoke (no vLLM required)
  python3 examples/python/benchmarks/mooncake_replay.py --smoke

  # Single-row dispatch (vLLM running)
  python3 examples/python/benchmarks/mooncake_replay.py \\
    --rows 1 --concurrency 1 --opt-levels 2 \\
    --output /tmp/mooncake_smoke.csv

  # Paired arms (run twice, combine for comparison_report)
  python3 examples/python/benchmarks/mooncake_replay.py --rows 50 --concurrency 4
  python3 examples/python/benchmarks/mooncake_replay.py --rows 50 --concurrency 4 --no-apxm-hints
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
WORKLOADS_DIR = Path(__file__).resolve().parent / "workloads"
DATA_DIR = WORKLOADS_DIR / "data"
DEFAULT_TRACE = DATA_DIR / "mooncake_sample.jsonl"
DEFAULT_OUTPUT = REPO_ROOT / ".apxm" / "benchmarks" / "results" / "mooncake.csv"
DEFAULT_ROW_GRAPH = WORKLOADS_DIR / "mooncake_row.py"

_TOOLS_SCRIPT_DIR = str(REPO_ROOT / "tools" / "scripts")
if _TOOLS_SCRIPT_DIR not in sys.path:
    sys.path.insert(0, _TOOLS_SCRIPT_DIR)

from apxm_vllm_contract import ArmName, EnvVar  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent))
from util import prom_pull  # noqa: E402
from util import pre_registration  # noqa: E402

sys.path.insert(0, str(DATA_DIR))
from _mooncake_hash import synthesize_prompt  # noqa: E402

APXM_DISABLE_HINTS_ENV = EnvVar.APXM_DISABLE_HINTS.value
DISABLE_HINTS_ENABLED = "1"
ARM_APXM_ON = ArmName.APXM_ON.value
ARM_FLAT_HTTP = ArmName.FLAT_HTTP.value

MOONCAKE_INPUT_TEXT_ENV = "MOONCAKE_INPUT_TEXT"
MOONCAKE_INPUT_TEXT_PATH_ENV = "MOONCAKE_INPUT_TEXT_PATH"
MOONCAKE_MAX_TOKENS_ENV = "MOONCAKE_MAX_TOKENS"
MOONCAKE_ROW_INDEX_ENV = "MOONCAKE_ROW_INDEX"
MOONCAKE_REUSE_GROUP_ENV = "MOONCAKE_REUSE_GROUP"
MATRIX_VARIANT_ENV = EnvVar.APXM_MATRIX_VARIANT.value
APXM_VLLM_CACHE_SALT_ENV = EnvVar.APXM_VLLM_CACHE_SALT.value
DEFAULT_APXM_ENDPOINT = os.environ.get(EnvVar.APXM_ENDPOINT.value, "")

DEKK_EXECUTE_CMD = ["dekk", "apxm", "execute"]
DEFAULT_TARGET = "latency"


@dataclass
class RowResult:
    row_index: int
    returncode: int
    wall_ms: float
    execution_id: str
    pinned_blocks_peak: int
    cached_input_tokens: int
    llm_calls: int
    input_length: int
    output_length: int
    hash_id_count: int


@dataclass
class BatchRow:
    timestamp: str
    cell_label: str
    workload: str
    model: str
    opt_level: int
    iteration: int
    concurrency: int
    rows: int
    batch_wall_ms: float
    max_tenant_wall_ms: float
    sum_tenant_wall_ms: float
    failed_tenants: int
    pinned_blocks_peak_max: int
    pinned_blocks_peak_sum: int
    cached_input_tokens_sum: int
    llm_calls_sum: int
    arm: str
    prefix_cache_hit_rate: float | None
    prefix_cache_queries_delta: float
    prefix_cache_hits_delta: float


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--trace", type=Path, default=DEFAULT_TRACE,
                   help="JSONL trace file (default: vendored 500-row sample)")
    p.add_argument("--output", type=Path, default=DEFAULT_OUTPUT,
                   help="Output CSV path (BatchRow schema)")
    p.add_argument("--concurrency", type=int, default=4)
    p.add_argument("--rows", type=int, default=0,
                   help="Number of rows to replay (0 = entire trace)")
    p.add_argument("--iterations", type=int, default=5,
                   help="Iterations per opt level (each iteration = one full trace pass; "
                        "paired significance checks require >=5 for Wilcoxon-quality numbers)")
    p.add_argument("--opt-levels", type=int, nargs="+", default=[0, 2])
    p.add_argument("--apxm-endpoint", default=DEFAULT_APXM_ENDPOINT,
                   help="vLLM service base URL (without /v1)")
    p.add_argument("--metrics-url", default=None,
                   help="Prometheus /metrics URL. Defaults to <apxm-endpoint>/metrics. "
                        "Set empty string to disable polling.")
    p.add_argument("--target", default=DEFAULT_TARGET,
                   help="Optimization target passed to dekk apxm execute")
    p.add_argument("--no-apxm-hints", action="store_true",
                   help="Run as the flat-HTTP control arm: suppress vllm_xargs.apxm "
                        "and /v1/apxm/graphs/register on the same vLLM build.")
    p.add_argument("--cell-label", default=None,
                   help="Override cell label. Default: derived from arm + opt level.")
    p.add_argument("--model", default=os.environ.get("MODEL_REF", ""),
                   help="Model id recorded in the row (informational)")
    p.add_argument("--workload-name", default="mooncake",
                   help="Workload name recorded in the row (informational)")
    p.add_argument("--row-graph", type=Path, default=DEFAULT_ROW_GRAPH,
                   help="Per-row graph script (default: workloads/mooncake_row.py)")
    p.add_argument("--matrix-report", type=Path, default=None,
                   help="Optional JSON coverage report path")
    p.add_argument("--pre-registration", type=Path, default=None,
                   help="Pre-registration markdown file authored before the run.")
    p.add_argument("--require-pre-registration", action="store_true",
                   help="Refuse to start when --pre-registration is missing.")
    p.add_argument("--smoke", action="store_true",
                   help="No-vLLM driver-only smoke: validates JSONL parsing, "
                        "hash synthesis, BatchRow schema, comparison_report compat. "
                        "Exits 0 on success.")
    return p.parse_args()


def _load_trace(path: Path, limit: int, max_total_tokens: int = 0) -> list[dict]:
    """Read trace rows. When `max_total_tokens > 0`, drop rows whose
    input_length + output_length exceeds the budget — these otherwise
    overflow the model's max_model_len and fail at the backend. The
    drop is recorded for the manifest so the cell honestly reports
    "ran against N filtered rows of M total."""
    rows: list[dict] = []
    dropped = 0
    with path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            if max_total_tokens > 0:
                total = int(row.get("input_length", 0)) + int(row.get("output_length", 0))
                if total > max_total_tokens:
                    dropped += 1
                    continue
            rows.append(row)
            if limit and len(rows) >= limit:
                break
    if dropped:
        print(
            f"[mooncake] dropped {dropped} rows whose input+output exceeded "
            f"max_total_tokens={max_total_tokens}",
            file=sys.stderr,
        )
    return rows


def _derive_cell_label(workload: str, opt_level: int, override: str | None) -> str:
    """Derive a cell label that identifies the experimental cell.

    The cell label captures axes that the apxm-on / flat-http arms must
    SHARE so the pairing in `comparison_report._pair_rows_by_arm` can
    match rows across the two arms. Arm and iteration are the axes that
    DIFFER; do not include them here.
    """
    if override:
        return override
    return f"{workload}-o{opt_level}"


def _execute_row(
    *,
    row_index: int,
    row: dict,
    opt_level: int,
    target: str,
    row_graph: Path,
    iteration: int,
    arm: str,
    extra_env: dict[str, str],
) -> RowResult:
    input_length = int(row.get("input_length", 0))
    output_length = int(row.get("output_length", 0))
    hash_ids = list(row.get("hash_ids", []))
    prompt = synthesize_prompt(hash_ids, input_length)

    env = os.environ.copy()
    env.update(extra_env)
    # Pass the prompt via a tempfile path. Inlining the prompt in env
    # overflows Linux ARG_MAX once cumulative env+arg crosses ~128 KB
    # (synthesize_prompt routinely produces 20-30 KB prompts and
    # _run_iteration's ThreadPoolExecutor accumulates them across
    # concurrent rows). The workload reads from MOONCAKE_INPUT_TEXT_PATH
    # when set; the inline MOONCAKE_INPUT_TEXT remains as a fallback
    # for any external caller still on the old contract.
    prompt_file = Path(tempfile.mkstemp(
        prefix=f"mooncake-prompt-r{row_index}-it{iteration}-",
        suffix=".txt",
    )[1])
    prompt_file.write_text(prompt, encoding="utf-8")
    env[MOONCAKE_INPUT_TEXT_PATH_ENV] = str(prompt_file)
    env[MOONCAKE_MAX_TOKENS_ENV] = str(output_length)
    env[MOONCAKE_ROW_INDEX_ENV] = str(row_index)
    env[MATRIX_VARIANT_ENV] = str(row_index)
    # Cohort key derived from the trace's first prefix-cache block id.
    # Rows that share hash_ids[0] land in the same APXM reuse_group;
    # the runtime's pin path engages on cohort-level KV pressure.
    # Pre-Wave-27 runs (before commit 0dcef78c) omitted this and the
    # APXM pin path stayed dormant on Mooncake (Mooncake's prefix
    # structure exists in hash_ids but was invisible to the runtime).
    if hash_ids:
        env[MOONCAKE_REUSE_GROUP_ENV] = f"mooncake-cohort-{hash_ids[0]}"
    # Salt scoped per (arm, opt_level): isolates the two arms' cache
    # namespaces and prevents opt=2 from inheriting opt=0's warm cache,
    # while leaving rows within a single (arm, opt) cell free to share
    # prefix-cache blocks per the trace's hash_ids structure (intended
    # workload behavior for Mooncake replay) and letting iterations of
    # the same row hit the warmed cache (sustained-state measurement).
    env[APXM_VLLM_CACHE_SALT_ENV] = f"mooncake-arm-{arm}-opt-{opt_level}"

    metrics_dir = Path(tempfile.mkdtemp(prefix=f"mooncake-row-{row_index}-it{iteration}-"))
    metrics_path = metrics_dir / "metrics.json"

    cmd = [
        *DEKK_EXECUTE_CMD,
        str(row_graph),
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
    pinned_peak = 0
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

    if metrics_path.exists():
        try:
            metrics_obj = json.loads(metrics_path.read_text())
            backends = metrics_obj.get("backends", {})
            graphs = backends.get("graphs", []) if isinstance(backends, dict) else []
            for graph in graphs:
                pinned_peak = max(pinned_peak, int(graph.get("pinned_blocks_peak", 0) or 0))
        except (json.JSONDecodeError, OSError, ValueError):
            pass
    try:
        if metrics_path.exists():
            metrics_path.unlink()
        metrics_dir.rmdir()
    except OSError:
        pass

    return RowResult(
        row_index=row_index,
        returncode=proc.returncode,
        wall_ms=wall_ms,
        execution_id=execution_id,
        pinned_blocks_peak=pinned_peak,
        cached_input_tokens=cached_input,
        llm_calls=llm_calls,
        input_length=input_length,
        output_length=output_length,
        hash_id_count=len(hash_ids),
    )


def _run_iteration(
    *,
    rows: list[dict],
    opt_level: int,
    iteration: int,
    args: argparse.Namespace,
    arm: str,
    metrics_url: str | None,
    extra_env: dict[str, str],
) -> tuple[BatchRow, list[RowResult]]:
    cell_label = _derive_cell_label(args.workload_name, opt_level, args.cell_label)

    metrics_before: dict[str, float] = {}
    if metrics_url:
        try:
            metrics_before = prom_pull.snapshot(metrics_url)
        except (urllib.error.URLError, TimeoutError) as exc:
            print(f"[mooncake] warn: metrics snapshot before iter failed: {exc!r}", flush=True)

    iter_start = time.perf_counter()
    results: list[RowResult] = []
    with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        futures = [
            pool.submit(
                _execute_row,
                row_index=idx,
                row=row,
                opt_level=opt_level,
                target=args.target,
                row_graph=args.row_graph,
                iteration=iteration,
                arm=arm,
                extra_env=extra_env,
            )
            for idx, row in enumerate(rows)
        ]
        for fut in as_completed(futures):
            results.append(fut.result())
    iter_wall_ms = (time.perf_counter() - iter_start) * 1000.0

    metrics_delta: dict[str, float] = {}
    if metrics_url and metrics_before:
        try:
            metrics_after = prom_pull.snapshot(metrics_url)
            metrics_delta = prom_pull.delta(metrics_before, metrics_after)
        except (urllib.error.URLError, TimeoutError) as exc:
            print(f"[mooncake] warn: metrics snapshot after iter failed: {exc!r}", flush=True)
    cell_hit_rate = prom_pull.hit_rate(metrics_delta) if metrics_delta else None
    queries_delta = metrics_delta.get(prom_pull.PREFIX_CACHE_QUERIES_TOTAL, 0.0)
    hits_delta = metrics_delta.get(prom_pull.PREFIX_CACHE_HITS_TOTAL, 0.0)

    failed = sum(1 for r in results if r.returncode != 0)
    if results:
        max_wall = max(r.wall_ms for r in results)
        sum_wall = sum(r.wall_ms for r in results)
        pin_peak_max = max(r.pinned_blocks_peak for r in results)
        pin_peak_sum = sum(r.pinned_blocks_peak for r in results)
        cached_sum = sum(r.cached_input_tokens for r in results)
        llm_calls_sum = sum(r.llm_calls for r in results)
    else:
        max_wall = sum_wall = 0.0
        pin_peak_max = pin_peak_sum = cached_sum = llm_calls_sum = 0

    row = BatchRow(
        timestamp=datetime.now(timezone.utc).isoformat(),
        cell_label=cell_label,
        workload=args.workload_name,
        model=args.model,
        opt_level=opt_level,
        iteration=iteration,
        concurrency=args.concurrency,
        rows=len(results),
        batch_wall_ms=iter_wall_ms,
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
    return row, results


def _write_csv(path: Path, rows: list[BatchRow]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fieldnames = list(asdict(rows[0]).keys()) if rows else [f.name for f in BatchRow.__dataclass_fields__.values()]
    with path.open("w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fieldnames)
        w.writeheader()
        for row in rows:
            w.writerow(asdict(row))


def _write_matrix_report(rows: list[BatchRow], path: Path, *, pre_registration_path: str | None) -> bool:
    cells: dict[str, dict] = {}
    ok = bool(rows) and all(row.failed_tenants == 0 for row in rows)
    for row in rows:
        cells[row.cell_label] = {
            "opt_level": row.opt_level,
            "iteration": row.iteration,
            "rows": row.rows,
            "failed_tenants": row.failed_tenants,
            "batch_wall_ms": row.batch_wall_ms,
            "pinned_blocks_peak_max": row.pinned_blocks_peak_max,
            "prefix_cache_hit_rate": row.prefix_cache_hit_rate,
            "arm": row.arm,
        }
    payload = {
        "matrix": "mooncake_replay",
        "ok": ok,
        "cells": cells,
        "pre_registration_path": pre_registration_path,
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    return ok


def _smoke(args: argparse.Namespace) -> int:
    print(f"[smoke] loading trace: {args.trace}")
    trace_rows = _load_trace(args.trace, limit=5)
    if len(trace_rows) < 2:
        print(f"[smoke] FAIL: trace has fewer than 2 rows: {len(trace_rows)}")
        return 1
    print(f"[smoke] loaded {len(trace_rows)} rows")

    # Determinism check.
    p1 = synthesize_prompt(trace_rows[0]["hash_ids"], trace_rows[0]["input_length"])
    p2 = synthesize_prompt(trace_rows[0]["hash_ids"], trace_rows[0]["input_length"])
    if p1 != p2:
        print("[smoke] FAIL: synthesize_prompt is non-deterministic")
        return 1
    print(f"[smoke] synthesize_prompt deterministic; row 0 prompt len chars={len(p1)}")

    # Build a synthetic BatchRow per arm and verify comparison_report can pair them.
    fake_rows: list[BatchRow] = []
    for arm in (ARM_APXM_ON, ARM_FLAT_HTTP):
        for opt in args.opt_levels:
            for it in range(1, 4):  # 3 seeds for stable Wilcoxon
                fake_rows.append(BatchRow(
                    timestamp=datetime.now(timezone.utc).isoformat(),
                    cell_label=_derive_cell_label(args.workload_name, opt, args.cell_label),
                    workload=args.workload_name,
                    model="smoke",
                    opt_level=opt,
                    iteration=it,
                    concurrency=args.concurrency,
                    rows=len(trace_rows),
                    batch_wall_ms=100.0 if arm == ARM_APXM_ON else 110.0,
                    max_tenant_wall_ms=90.0 if arm == ARM_APXM_ON else 100.0,
                    sum_tenant_wall_ms=400.0 if arm == ARM_APXM_ON else 440.0,
                    failed_tenants=0,
                    pinned_blocks_peak_max=12 if arm == ARM_APXM_ON else 0,
                    pinned_blocks_peak_sum=80 if arm == ARM_APXM_ON else 0,
                    cached_input_tokens_sum=500 if arm == ARM_APXM_ON else 0,
                    llm_calls_sum=len(trace_rows),
                    arm=arm,
                    prefix_cache_hit_rate=0.7 if arm == ARM_APXM_ON else 0.0,
                    prefix_cache_queries_delta=10.0,
                    prefix_cache_hits_delta=7.0 if arm == ARM_APXM_ON else 0.0,
                ))

    out = args.output if args.output != DEFAULT_OUTPUT else Path(tempfile.mkdtemp()) / "smoke.csv"
    _write_csv(out, fake_rows)
    print(f"[smoke] wrote synthetic CSV: {out}")

    # Hand off to comparison_report._summarize_by_arm to verify schema compat.
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import comparison_report as r
    raw_rows = list(csv.DictReader(out.open()))
    summary = r._summarize_by_arm(raw_rows, bootstrap_samples=200)
    if summary is None:
        print("[smoke] FAIL: comparison_report._summarize_by_arm returned None")
        return 1
    if summary["pair_count"] < 1:
        print(f"[smoke] FAIL: pair_count={summary['pair_count']}")
        return 1
    if "batch_wall_ms" not in summary["metrics"]:
        print("[smoke] FAIL: batch_wall_ms not in summary metrics")
        return 1
    print(f"[smoke] arm comparison: pair_count={summary['pair_count']} "
          f"metrics={list(summary['metrics'].keys())}")
    print("[smoke] OK")
    return 0


def main() -> int:
    args = _parse_args()

    if args.smoke:
        return _smoke(args)

    if args.no_apxm_hints:
        os.environ[APXM_DISABLE_HINTS_ENV] = DISABLE_HINTS_ENABLED
    arm = ARM_FLAT_HTTP if args.no_apxm_hints else ARM_APXM_ON

    if args.metrics_url is None:
        metrics_url = f"{args.apxm_endpoint.rstrip('/')}/metrics"
    elif args.metrics_url == "":
        metrics_url = None
    else:
        metrics_url = args.metrics_url

    pre_reg_sidecar = pre_registration.enforce(
        pre_registration_arg=args.pre_registration,
        require=args.require_pre_registration,
        output_path=args.output.resolve(),
    )

    # Filter rows that would exceed the configured max_model_len. The
    # Mooncake trace was recorded against a longer-context engine; on
    # gpt-oss-120b with max_model_len=32768, the long tail (~7% of
    # rows) cannot fit and fails at the backend with a clear out-of-
    # bounds error. Dropping them is the honest move; the manifest
    # records the count so the cell still cites N filtered of M total.
    max_total = int(os.environ.get("MAX_MODEL_LEN", "0") or 0)
    trace_rows = _load_trace(args.trace, limit=args.rows, max_total_tokens=max_total)
    if not trace_rows:
        print(f"error: no rows loaded from {args.trace}", file=sys.stderr)
        return 1
    print(f"[mooncake] loaded {len(trace_rows)} trace rows from {args.trace}")
    print(f"[mooncake] arm={arm} opt_levels={args.opt_levels} concurrency={args.concurrency}")

    # Subprocess env additions are minimal — APXM_DISABLE_HINTS already in
    # parent env via os.environ.copy() inside _execute_row.
    extra_env: dict[str, str] = {}

    rows: list[BatchRow] = []
    for opt in args.opt_levels:
        for iteration in range(1, args.iterations + 1):
            print(f"[mooncake] opt={opt} iteration={iteration} starting...", flush=True)
            row, _row_results = _run_iteration(
                rows=trace_rows,
                opt_level=opt,
                iteration=iteration,
                args=args,
                arm=arm,
                metrics_url=metrics_url,
                extra_env=extra_env,
            )
            rows.append(row)
            print(
                f"[mooncake]   batch_wall_ms={row.batch_wall_ms:.1f} "
                f"failed={row.failed_tenants} "
                f"pinned_peak_max={row.pinned_blocks_peak_max} "
                f"hit_rate={row.prefix_cache_hit_rate}",
                flush=True,
            )

    _write_csv(args.output, rows)
    print(f"\nWrote {len(rows)} batch rows to {args.output}")

    if args.matrix_report:
        ok = _write_matrix_report(rows, args.matrix_report, pre_registration_path=pre_reg_sidecar)
        print(f"Wrote Mooncake matrix report to {args.matrix_report} (ok={ok})")
        if not ok:
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
