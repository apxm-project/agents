#!/usr/bin/env python3
"""Combine accepted APXM priority-lane runs for paper-level sensitivity."""

from __future__ import annotations

import argparse
import csv
import json
import math
import random
import statistics
from pathlib import Path
from typing import Callable


def read_rows(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8") as fh:
        return list(csv.DictReader(fh))


def as_float(row: dict[str, str], key: str) -> float:
    try:
        return float(row.get(key, "") or 0)
    except ValueError:
        return 0.0


def percentile(values: list[float], q: float) -> float:
    ordered = sorted(values)
    idx = math.ceil((len(ordered) - 1) * q)
    return ordered[max(0, min(idx, len(ordered) - 1))]


def ratio(num: float, den: float) -> float:
    return math.nan if den <= 0 else num / den


def paired(
    left_rows: list[dict[str, str]],
    right_rows: list[dict[str, str]],
    keys: tuple[str, ...],
) -> list[tuple[dict[str, str], dict[str, str]]]:
    right_by_key = {tuple(row.get(key, "") for key in keys): row for row in right_rows}
    out = []
    for left in left_rows:
        key = tuple(left.get(item, "") for item in keys)
        right = right_by_key.get(key)
        if right is not None:
            out.append((left, right))
    return out


def bootstrap_pair_ratio(
    pairs: list[tuple[dict[str, str], dict[str, str]]],
    key: str,
    estimator: Callable[[list[float]], float],
    *,
    n: int,
    seed: int,
) -> dict[str, float]:
    samples = [
        (as_float(apxm, key), as_float(flat, key))
        for apxm, flat in pairs
        if as_float(apxm, key) > 0 and as_float(flat, key) > 0
    ]
    point = ratio(
        estimator([item[0] for item in samples]),
        estimator([item[1] for item in samples]),
    )
    rng = random.Random(seed)
    draws = []
    for _ in range(n):
        sample = [rng.choice(samples) for _ in samples]
        den = estimator([item[1] for item in sample])
        if den > 0:
            draws.append(estimator([item[0] for item in sample]) / den)
    draws.sort()
    return {
        "ratio": point,
        "ci_low": draws[int(0.025 * len(draws))],
        "ci_high": draws[int(0.975 * len(draws))],
    }


def bootstrap_sum_ratio(
    pairs: list[tuple[dict[str, str], dict[str, str]]],
    key: str,
    *,
    n: int,
    seed: int,
) -> dict[str, float]:
    samples = [
        (as_float(apxm, key), as_float(flat, key))
        for apxm, flat in pairs
        if as_float(apxm, key) > 0 and as_float(flat, key) > 0
    ]
    point = ratio(sum(item[0] for item in samples), sum(item[1] for item in samples))
    rng = random.Random(seed)
    draws = []
    for _ in range(n):
        sample = [rng.choice(samples) for _ in samples]
        den = sum(item[1] for item in sample)
        if den > 0:
            draws.append(sum(item[0] for item in sample) / den)
    draws.sort()
    return {
        "ratio": point,
        "ci_low": draws[int(0.025 * len(draws))],
        "ci_high": draws[int(0.975 * len(draws))],
    }


def fmt(value: float) -> str:
    return "nan" if math.isnan(value) else f"{value:.3f}"


def load_combined(run_dirs: list[Path]) -> tuple[list[dict[str, str]], list[dict[str, str]], list[dict[str, str]], list[dict[str, str]]]:
    apxm_tenants: list[dict[str, str]] = []
    flat_tenants: list[dict[str, str]] = []
    apxm_batches: list[dict[str, str]] = []
    flat_batches: list[dict[str, str]] = []
    for idx, run_dir in enumerate(run_dirs, start=1):
        run_label = run_dir.name
        for source, target in (
            ("priority.apxm-on.tenants.csv", apxm_tenants),
            ("priority.flat-http.tenants.csv", flat_tenants),
        ):
            for row in read_rows(run_dir / source):
                row = dict(row)
                row["run"] = run_label
                row["batch_unit"] = f"r{idx}-i{row.get('iteration', '')}"
                target.append(row)
        for source, target in (
            ("priority.apxm-on.csv", apxm_batches),
            ("priority.flat-http.csv", flat_batches),
        ):
            for row in read_rows(run_dir / source):
                row = dict(row)
                row["run"] = run_label
                row["batch_unit"] = f"r{idx}-i{row.get('iteration', '')}"
                target.append(row)
    return apxm_tenants, flat_tenants, apxm_batches, flat_batches


def write_batch_csv(path: Path, batch_pairs: list[tuple[dict[str, str], dict[str, str]]]) -> None:
    fieldnames = [
        "batch_unit",
        "run",
        "iteration",
        "apxm_focus_node_finish_ms_mean",
        "flat_focus_node_finish_ms_mean",
        "focus_node_finish_ms_mean_ratio",
        "apxm_batch_wall_ms",
        "flat_batch_wall_ms",
        "batch_wall_ms_ratio",
        "apxm_sum_tenant_wall_ms",
        "flat_sum_tenant_wall_ms",
        "sum_tenant_wall_ms_ratio",
    ]
    with path.open("w", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=fieldnames)
        writer.writeheader()
        for apxm, flat in batch_pairs:
            writer.writerow({
                "batch_unit": apxm["batch_unit"],
                "run": apxm["run"],
                "iteration": apxm.get("iteration", ""),
                "apxm_focus_node_finish_ms_mean": as_float(apxm, "focus_node_finish_ms_mean"),
                "flat_focus_node_finish_ms_mean": as_float(flat, "focus_node_finish_ms_mean"),
                "focus_node_finish_ms_mean_ratio": ratio(
                    as_float(apxm, "focus_node_finish_ms_mean"),
                    as_float(flat, "focus_node_finish_ms_mean"),
                ),
                "apxm_batch_wall_ms": as_float(apxm, "batch_wall_ms"),
                "flat_batch_wall_ms": as_float(flat, "batch_wall_ms"),
                "batch_wall_ms_ratio": ratio(
                    as_float(apxm, "batch_wall_ms"),
                    as_float(flat, "batch_wall_ms"),
                ),
                "apxm_sum_tenant_wall_ms": as_float(apxm, "sum_tenant_wall_ms"),
                "flat_sum_tenant_wall_ms": as_float(flat, "sum_tenant_wall_ms"),
                "sum_tenant_wall_ms_ratio": ratio(
                    as_float(apxm, "sum_tenant_wall_ms"),
                    as_float(flat, "sum_tenant_wall_ms"),
                ),
            })


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--bootstrap-n", type=int, default=10000)
    parser.add_argument("--seed", type=int, default=260522)
    parser.add_argument("run_dirs", nargs="+", type=Path)
    args = parser.parse_args()

    apxm_tenants, flat_tenants, apxm_batches, flat_batches = load_combined(args.run_dirs)
    tenant_pairs = paired(apxm_tenants, flat_tenants, ("batch_unit", "variant"))
    batch_pairs = paired(apxm_batches, flat_batches, ("batch_unit",))

    tenant_valid = [
        (as_float(apxm, "focus_node_finish_ms"), as_float(flat, "focus_node_finish_ms"))
        for apxm, flat in tenant_pairs
        if as_float(apxm, "focus_node_finish_ms") > 0 and as_float(flat, "focus_node_finish_ms") > 0
    ]
    focus_wins = sum(apxm < flat for apxm, flat in tenant_valid)
    report = {
        "run_dirs": [str(path) for path in args.run_dirs],
        "paired_tenants": len(tenant_pairs),
        "paired_batches": len(batch_pairs),
        "tenant_level": {
            "focus_node_finish_ms_mean": bootstrap_pair_ratio(
                tenant_pairs,
                "focus_node_finish_ms",
                statistics.mean,
                n=args.bootstrap_n,
                seed=args.seed,
            ),
            "focus_node_finish_ms_p95": bootstrap_pair_ratio(
                tenant_pairs,
                "focus_node_finish_ms",
                lambda values: percentile(values, 0.95),
                n=args.bootstrap_n,
                seed=args.seed + 1,
            ),
            "focus_win_rate": {
                "apxm_wins": focus_wins,
                "pairs": len(tenant_valid),
                "win_rate": ratio(focus_wins, len(tenant_valid)),
            },
        },
        "batch_level": {
            "focus_node_finish_ms_mean": bootstrap_sum_ratio(
                batch_pairs,
                "focus_node_finish_ms_mean",
                n=args.bootstrap_n,
                seed=args.seed + 2,
            ),
            "batch_wall_ms": bootstrap_sum_ratio(
                batch_pairs,
                "batch_wall_ms",
                n=args.bootstrap_n,
                seed=args.seed + 3,
            ),
            "sum_tenant_wall_ms": bootstrap_sum_ratio(
                batch_pairs,
                "sum_tenant_wall_ms",
                n=args.bootstrap_n,
                seed=args.seed + 4,
            ),
        },
    }

    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "combined-priority-lane-analysis.json").write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    write_batch_csv(args.output_dir / "combined-priority-lane-batches.csv", batch_pairs)
    focus = report["tenant_level"]["focus_node_finish_ms_mean"]
    p95 = report["tenant_level"]["focus_node_finish_ms_p95"]
    batch_focus = report["batch_level"]["focus_node_finish_ms_mean"]
    batch_wall = report["batch_level"]["batch_wall_ms"]
    sum_wall = report["batch_level"]["sum_tenant_wall_ms"]
    text = f"""# Combined APXM Priority-Lane Fresh-Service Analysis

Run count: `{len(args.run_dirs)}`
Paired tenant rows: `{report['paired_tenants']}`
Independent batch units: `{report['paired_batches']}`

## Tenant-Level Result

- Mean focus-node APXM/flat: `{fmt(focus['ratio'])}`, 95% CI `[{fmt(focus['ci_low'])}, {fmt(focus['ci_high'])}]`
- p95 focus-node APXM/flat: `{fmt(p95['ratio'])}`, 95% CI `[{fmt(p95['ci_low'])}, {fmt(p95['ci_high'])}]`
- Tenant focus win rate: `{focus_wins}/{len(tenant_valid)}`

## Batch-Level Sensitivity

| Metric | APXM/flat | 95% bootstrap CI |
|---|---:|---:|
| Focus-node mean | `{fmt(batch_focus['ratio'])}` | `[{fmt(batch_focus['ci_low'])}, {fmt(batch_focus['ci_high'])}]` |
| Batch wall | `{fmt(batch_wall['ratio'])}` | `[{fmt(batch_wall['ci_low'])}, {fmt(batch_wall['ci_high'])}]` |
| Sum tenant wall | `{fmt(sum_wall['ratio'])}` | `[{fmt(sum_wall['ci_low'])}, {fmt(sum_wall['ci_high'])}]` |

Interpretation: this combines fresh-service repeats as independent batch units.
The supported claim remains priority-lane focus latency under contention.
"""
    (args.output_dir / "combined-priority-lane-analysis.md").write_text(text, encoding="utf-8")
    print(json.dumps({
        "json": str(args.output_dir / "combined-priority-lane-analysis.json"),
        "csv": str(args.output_dir / "combined-priority-lane-batches.csv"),
        "markdown": str(args.output_dir / "combined-priority-lane-analysis.md"),
    }, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
