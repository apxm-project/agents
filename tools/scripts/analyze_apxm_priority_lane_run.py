#!/usr/bin/env python3
"""Add batch-level and tradeoff analysis to an APXM priority-lane run."""

from __future__ import annotations

import argparse
import csv
import json
import math
import random
import statistics
from pathlib import Path
from typing import Callable, Iterable


def _read_rows(path: Path) -> list[dict[str, str]]:
    with path.open(newline="") as fh:
        return list(csv.DictReader(fh))


def _float(row: dict[str, str], key: str) -> float:
    try:
        return float(row.get(key, "") or 0)
    except ValueError:
        return 0.0


def _int(row: dict[str, str], key: str) -> int:
    try:
        return int(float(row.get(key, "") or 0))
    except ValueError:
        return 0


def _percentile(values: list[float], q: float) -> float:
    if not values:
        return math.nan
    ordered = sorted(values)
    idx = math.ceil((len(ordered) - 1) * q)
    return ordered[max(0, min(idx, len(ordered) - 1))]


def _ratio(numerator: float, denominator: float) -> float:
    if denominator <= 0:
        return math.nan
    return numerator / denominator


def _paired_by(
    left_rows: Iterable[dict[str, str]],
    right_rows: Iterable[dict[str, str]],
    keys: tuple[str, ...],
) -> list[tuple[dict[str, str], dict[str, str]]]:
    right_by_key = {
        tuple(row.get(key, "") for key in keys): row
        for row in right_rows
    }
    pairs = []
    for row in left_rows:
        key = tuple(row.get(item, "") for item in keys)
        other = right_by_key.get(key)
        if other is not None:
            pairs.append((row, other))
    return pairs


def _bootstrap_pair_ratio(
    pairs: list[tuple[dict[str, str], dict[str, str]]],
    key: str,
    estimator: Callable[[list[float]], float],
    *,
    n: int,
    seed: int,
) -> dict[str, float]:
    samples = [
        (_float(apxm, key), _float(flat, key))
        for apxm, flat in pairs
        if _float(apxm, key) > 0 and _float(flat, key) > 0
    ]
    if not samples:
        return {"ratio": math.nan, "ci_low": math.nan, "ci_high": math.nan}
    apxm_values = [item[0] for item in samples]
    flat_values = [item[1] for item in samples]
    point = _ratio(estimator(apxm_values), estimator(flat_values))

    rng = random.Random(seed)
    draws = []
    for _ in range(n):
        sample = [rng.choice(samples) for _ in samples]
        apxm_draw = [item[0] for item in sample]
        flat_draw = [item[1] for item in sample]
        denominator = estimator(flat_draw)
        if denominator > 0:
            draws.append(estimator(apxm_draw) / denominator)
    draws.sort()
    return {
        "ratio": point,
        "ci_low": draws[int(0.025 * len(draws))],
        "ci_high": draws[int(0.975 * len(draws))],
    }


def _bootstrap_sum_ratio(
    pairs: list[tuple[dict[str, str], dict[str, str]]],
    key: str,
    *,
    n: int,
    seed: int,
) -> dict[str, float]:
    samples = [
        (_float(apxm, key), _float(flat, key))
        for apxm, flat in pairs
        if _float(apxm, key) > 0 and _float(flat, key) > 0
    ]
    if not samples:
        return {"ratio": math.nan, "ci_low": math.nan, "ci_high": math.nan}
    point = _ratio(sum(item[0] for item in samples), sum(item[1] for item in samples))

    rng = random.Random(seed)
    draws = []
    for _ in range(n):
        sample = [rng.choice(samples) for _ in samples]
        denominator = sum(item[1] for item in sample)
        if denominator > 0:
            draws.append(sum(item[0] for item in sample) / denominator)
    draws.sort()
    return {
        "ratio": point,
        "ci_low": draws[int(0.025 * len(draws))],
        "ci_high": draws[int(0.975 * len(draws))],
    }


def _tenant_win_rates(
    tenant_pairs: list[tuple[dict[str, str], dict[str, str]]],
) -> dict[str, dict[str, float]]:
    out = {}
    for key in ("focus_node_finish_ms", "critical_path_finish_ms", "wall_ms"):
        valid = [
            (_float(apxm, key), _float(flat, key))
            for apxm, flat in tenant_pairs
            if _float(apxm, key) > 0 and _float(flat, key) > 0
        ]
        wins = sum(apxm < flat for apxm, flat in valid)
        ties = sum(apxm == flat for apxm, flat in valid)
        out[key] = {
            "pairs": len(valid),
            "apxm_wins": wins,
            "ties": ties,
            "win_rate": _ratio(wins, len(valid)),
        }
    return out


def _batch_rows(
    batch_pairs: list[tuple[dict[str, str], dict[str, str]]],
) -> list[dict[str, float | int]]:
    rows: list[dict[str, float | int]] = []
    for apxm, flat in sorted(batch_pairs, key=lambda pair: _int(pair[0], "iteration")):
        item: dict[str, float | int] = {"iteration": _int(apxm, "iteration")}
        for key in (
            "focus_node_finish_ms_mean",
            "focus_node_finish_ms_max",
            "batch_wall_ms",
            "sum_tenant_wall_ms",
            "max_tenant_wall_ms",
            "critical_path_finish_ms_mean",
            "prefix_cache_hit_rate",
        ):
            apxm_value = _float(apxm, key)
            flat_value = _float(flat, key)
            item[f"apxm_{key}"] = apxm_value
            item[f"flat_{key}"] = flat_value
            item[f"{key}_ratio"] = _ratio(apxm_value, flat_value)
        rows.append(item)
    return rows


def _leave_one_out(
    batch_pairs: list[tuple[dict[str, str], dict[str, str]]],
    key: str,
) -> list[dict[str, float | int]]:
    out = []
    for omitted in sorted({_int(apxm, "iteration") for apxm, _ in batch_pairs}):
        kept = [
            (apxm, flat)
            for apxm, flat in batch_pairs
            if _int(apxm, "iteration") != omitted
        ]
        out.append({
            "omitted_iteration": omitted,
            "ratio": _ratio(
                sum(_float(apxm, key) for apxm, _ in kept),
                sum(_float(flat, key) for _, flat in kept),
            ),
        })
    return out


def _write_batch_csv(path: Path, rows: list[dict[str, float | int]]) -> None:
    if not rows:
        return
    fieldnames = list(rows[0].keys())
    with path.open("w", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


def _fmt(value: float) -> str:
    if math.isnan(value):
        return "nan"
    return f"{value:.3f}"


def _write_markdown(path: Path, report: dict[str, object]) -> None:
    tenant = report["tenant_level"]  # type: ignore[index]
    batch = report["batch_level"]  # type: ignore[index]
    tradeoff = report["tradeoffs"]  # type: ignore[index]
    focus = tenant["bootstrap"]["focus_node_finish_ms_mean"]  # type: ignore[index]
    p95 = tenant["bootstrap"]["focus_node_finish_ms_p95"]  # type: ignore[index]
    batch_focus = batch["bootstrap"]["focus_node_finish_ms_mean"]  # type: ignore[index]
    batch_wall = batch["bootstrap"]["batch_wall_ms"]  # type: ignore[index]
    sum_wall = batch["bootstrap"]["sum_tenant_wall_ms"]  # type: ignore[index]

    if batch_focus["ci_high"] < 1.0:
        batch_interpretation = (
            "The batch-level sensitivity also supports an APXM win, but the "
            f"run still has only `{batch['paired_batches']}` independent "
            "iterations."
        )
    else:
        batch_interpretation = (
            "The tenant-level result is promotion-safe for the current claim, "
            "but the batch-level CI is wide because this run has only "
            f"`{batch['paired_batches']}` independent iterations."
        )

    text = f"""# APXM Priority-Lane Enhanced Analysis

Run: `{report['run_dir']}`

## Headline

Tenant-level focus-node finish time remains the strongest positive result:
mean APXM/flat `{_fmt(focus['ratio'])}` with CI `[{_fmt(focus['ci_low'])}, {_fmt(focus['ci_high'])}]`;
p95 APXM/flat `{_fmt(p95['ratio'])}` with CI `[{_fmt(p95['ci_low'])}, {_fmt(p95['ci_high'])}]`.

## Batch-Level Sensitivity

The batch-level analysis treats each iteration as one independent unit. This is
the conservative sensitivity check reviewers will ask for.

| Metric | APXM/flat | 95% bootstrap CI |
|---|---:|---:|
| Focus mean across batches | `{_fmt(batch_focus['ratio'])}` | `[{_fmt(batch_focus['ci_low'])}, {_fmt(batch_focus['ci_high'])}]` |
| Batch wall | `{_fmt(batch_wall['ratio'])}` | `[{_fmt(batch_wall['ci_low'])}, {_fmt(batch_wall['ci_high'])}]` |
| Sum tenant wall | `{_fmt(sum_wall['ratio'])}` | `[{_fmt(sum_wall['ci_low'])}, {_fmt(sum_wall['ci_high'])}]` |

Interpretation: {batch_interpretation} Treat this as a publication-strength
signal: rerun across fresh services/nodes before paper submission, but do not
discard the current result.

## Tradeoffs

- Tenant focus-node win rate: `{tradeoff['tenant_focus_win_rate']['apxm_wins']}/{tradeoff['tenant_focus_win_rate']['pairs']}`.
- Tenant wall-time win rate: `{tradeoff['tenant_wall_win_rate']['apxm_wins']}/{tradeoff['tenant_wall_win_rate']['pairs']}`.
- Aggregate batch-wall ratio: `{_fmt(batch_wall['ratio'])}`.
- Aggregate sum-tenant-wall ratio: `{_fmt(sum_wall['ratio'])}`.

The result is best described as a priority-lane win under contention, not as a
universal warm-service batch-wall win.
"""
    path.write_text(text)


def analyze(run_dir: Path, *, bootstrap_n: int, seed: int) -> dict[str, object]:
    apxm_tenants = _read_rows(run_dir / "priority.apxm-on.tenants.csv")
    flat_tenants = _read_rows(run_dir / "priority.flat-http.tenants.csv")
    apxm_batches = _read_rows(run_dir / "priority.apxm-on.csv")
    flat_batches = _read_rows(run_dir / "priority.flat-http.csv")

    tenant_pairs = _paired_by(apxm_tenants, flat_tenants, ("iteration", "variant"))
    batch_pairs = _paired_by(apxm_batches, flat_batches, ("iteration",))
    batch_detail = _batch_rows(batch_pairs)
    wins = _tenant_win_rates(tenant_pairs)

    report: dict[str, object] = {
        "run_dir": str(run_dir),
        "tenant_level": {
            "paired_tenants": len(tenant_pairs),
            "bootstrap": {
                "focus_node_finish_ms_mean": _bootstrap_pair_ratio(
                    tenant_pairs,
                    "focus_node_finish_ms",
                    statistics.mean,
                    n=bootstrap_n,
                    seed=seed,
                ),
                "focus_node_finish_ms_p95": _bootstrap_pair_ratio(
                    tenant_pairs,
                    "focus_node_finish_ms",
                    lambda values: _percentile(values, 0.95),
                    n=bootstrap_n,
                    seed=seed + 1,
                ),
            },
            "win_rates": wins,
        },
        "batch_level": {
            "paired_batches": len(batch_pairs),
            "bootstrap": {
                "focus_node_finish_ms_mean": _bootstrap_sum_ratio(
                    batch_pairs,
                    "focus_node_finish_ms_mean",
                    n=bootstrap_n,
                    seed=seed + 2,
                ),
                "batch_wall_ms": _bootstrap_sum_ratio(
                    batch_pairs,
                    "batch_wall_ms",
                    n=bootstrap_n,
                    seed=seed + 3,
                ),
                "sum_tenant_wall_ms": _bootstrap_sum_ratio(
                    batch_pairs,
                    "sum_tenant_wall_ms",
                    n=bootstrap_n,
                    seed=seed + 4,
                ),
                "max_tenant_wall_ms": _bootstrap_sum_ratio(
                    batch_pairs,
                    "max_tenant_wall_ms",
                    n=bootstrap_n,
                    seed=seed + 5,
                ),
            },
            "leave_one_out": {
                "focus_node_finish_ms_mean": _leave_one_out(
                    batch_pairs, "focus_node_finish_ms_mean"
                ),
                "batch_wall_ms": _leave_one_out(batch_pairs, "batch_wall_ms"),
            },
        },
        "tradeoffs": {
            "tenant_focus_win_rate": wins["focus_node_finish_ms"],
            "tenant_wall_win_rate": wins["wall_ms"],
            "tenant_critical_path_win_rate": wins["critical_path_finish_ms"],
        },
        "interpretation": {
            "safe_claim": (
                "Priority-lane focus latency improves on tenant-level metrics "
                "under background queue contention."
            ),
            "caveat": (
                "Batch-level sensitivity is wide with the current number of "
                "independent iterations; repeat across fresh services before "
                "paper submission."
            ),
        },
    }
    return report | {"batch_rows": batch_detail}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--bootstrap-n", type=int, default=10000)
    parser.add_argument("--seed", type=int, default=260521)
    args = parser.parse_args()

    report_with_rows = analyze(args.run_dir, bootstrap_n=args.bootstrap_n, seed=args.seed)
    batch_rows = report_with_rows.pop("batch_rows")

    args.run_dir.mkdir(parents=True, exist_ok=True)
    json_path = args.run_dir / "priority-lane-enhanced-analysis.json"
    csv_path = args.run_dir / "priority-lane-batch-tradeoffs.csv"
    md_path = args.run_dir / "priority-lane-enhanced-analysis.md"
    json_path.write_text(json.dumps(report_with_rows, indent=2, sort_keys=True) + "\n")
    _write_batch_csv(csv_path, batch_rows)  # type: ignore[arg-type]
    _write_markdown(md_path, report_with_rows)
    print(json.dumps({
        "json": str(json_path),
        "csv": str(csv_path),
        "markdown": str(md_path),
    }, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
