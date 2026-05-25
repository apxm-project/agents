#!/usr/bin/env python3
"""Summarize paired APXM priority-lane tenant results."""

from __future__ import annotations

import argparse
import csv
import json
import math
import random
import statistics
from pathlib import Path
from typing import Callable


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


def _paired_rows(
    apxm_rows: list[dict[str, str]],
    flat_rows: list[dict[str, str]],
) -> list[tuple[dict[str, str], dict[str, str]]]:
    flat_by_key = {
        (row.get("iteration", ""), row.get("variant", "")): row
        for row in flat_rows
    }
    pairs = []
    for row in apxm_rows:
        key = (row.get("iteration", ""), row.get("variant", ""))
        flat = flat_by_key.get(key)
        if flat is not None:
            pairs.append((row, flat))
    return pairs


def _bootstrap_ratio(
    pairs: list[tuple[dict[str, str], dict[str, str]]],
    key: str,
    estimator: Callable[[list[float]], float],
    *,
    n: int = 10000,
    seed: int = 260521,
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
    ratio = estimator(apxm_values) / estimator(flat_values)

    rng = random.Random(seed)
    ratios = []
    for _ in range(n):
        draw = [rng.choice(samples) for _ in samples]
        apxm_draw = [item[0] for item in draw]
        flat_draw = [item[1] for item in draw]
        denominator = estimator(flat_draw)
        if denominator > 0:
            ratios.append(estimator(apxm_draw) / denominator)
    ratios.sort()
    return {
        "ratio": ratio,
        "ci_low": ratios[int(0.025 * len(ratios))],
        "ci_high": ratios[int(0.975 * len(ratios))],
    }


def _arm_summary(rows: list[dict[str, str]]) -> dict[str, object]:
    focus = [_float(row, "focus_node_finish_ms") for row in rows]
    focus = [value for value in focus if value > 0]
    background = [_float(row, "wall_ms") for row in rows]
    background = [value for value in background if value > 0]
    return {
        "rows": len(rows),
        "failed_tenants": sum(1 for row in rows if _int(row, "returncode") != 0),
        "dispatch_fallback_tenants": sum(
            1 for row in rows
            if (row.get("dispatch_fallback_triggered", "").lower() == "true")
        ),
        "focus_node_finish_ms_mean": statistics.mean(focus) if focus else math.nan,
        "focus_node_finish_ms_p50": statistics.median(focus) if focus else math.nan,
        "focus_node_finish_ms_p95": _percentile(focus, 0.95),
        "focus_node_finish_ms_max": max(focus, default=math.nan),
        "focus_node_queue_wait_ms_p95": _percentile(
            [_float(row, "focus_node_queue_wait_ms") for row in rows], 0.95
        ),
        "tenant_wall_ms_p95": _percentile(background, 0.95),
        "fields_honored": sorted({
            field
            for row in rows
            for segment in row.get("fields_honored", "").split(";")
            for field in segment.partition(":")[2].split("|")
            if field
        }),
    }


def _verdict(report: dict[str, object]) -> str:
    gates = report["gates"]
    if all(gates.values()):
        ratio = report["bootstrap"]["focus_node_finish_ms_mean"]["ratio"]
        return f"APXM priority-lane win; mean ratio={ratio:.3f}"
    failed = [name for name, ok in gates.items() if not ok]
    return "not promoted; failed gates: " + ", ".join(failed)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apxm-tenants", type=Path, required=True)
    parser.add_argument("--flat-tenants", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--summary-csv", type=Path, required=True)
    args = parser.parse_args()

    apxm_rows = _read_rows(args.apxm_tenants)
    flat_rows = _read_rows(args.flat_tenants)
    pairs = _paired_rows(apxm_rows, flat_rows)

    bootstrap = {
        "focus_node_finish_ms_mean": _bootstrap_ratio(
            pairs, "focus_node_finish_ms", statistics.mean
        ),
        "focus_node_finish_ms_p95": _bootstrap_ratio(
            pairs, "focus_node_finish_ms", lambda values: _percentile(values, 0.95)
        ),
    }
    apxm_summary = _arm_summary(apxm_rows)
    flat_summary = _arm_summary(flat_rows)

    gates = {
        "paired_rows_present": len(pairs) > 0,
        "zero_failures": (
            apxm_summary["failed_tenants"] == 0
            and flat_summary["failed_tenants"] == 0
        ),
        "apxm_no_dispatch_fallback": apxm_summary["dispatch_fallback_tenants"] == 0,
        "apxm_priority_honored": "priority" in apxm_summary["fields_honored"],
        "focus_metric_present": (
            not math.isnan(apxm_summary["focus_node_finish_ms_p95"])
            and not math.isnan(flat_summary["focus_node_finish_ms_p95"])
        ),
        "mean_ratio_ci_lt_0_95": (
            bootstrap["focus_node_finish_ms_mean"]["ci_high"] < 0.95
        ),
        "p95_ratio_ci_lt_0_90": (
            bootstrap["focus_node_finish_ms_p95"]["ci_high"] < 0.90
        ),
        "absolute_p95_win_ge_1000ms": (
            flat_summary["focus_node_finish_ms_p95"]
            - apxm_summary["focus_node_finish_ms_p95"]
            >= 1000.0
        ),
    }

    report = {
        "scenario": "apxm-priority-lane",
        "apxm_tenants": str(args.apxm_tenants),
        "flat_tenants": str(args.flat_tenants),
        "paired_rows": len(pairs),
        "arms": {
            "apxm-on": apxm_summary,
            "flat-http": flat_summary,
        },
        "bootstrap": bootstrap,
        "gates": gates,
    }
    report["verdict"] = _verdict(report)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")

    args.summary_csv.parent.mkdir(parents=True, exist_ok=True)
    with args.summary_csv.open("w", newline="") as fh:
        fieldnames = [
            "scenario",
            "paired_rows",
            "apxm_focus_p95_ms",
            "flat_focus_p95_ms",
            "mean_ratio",
            "mean_ci_low",
            "mean_ci_high",
            "p95_ratio",
            "p95_ci_low",
            "p95_ci_high",
            "verdict",
        ]
        writer = csv.DictWriter(fh, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerow({
            "scenario": "apxm-priority-lane",
            "paired_rows": len(pairs),
            "apxm_focus_p95_ms": f"{apxm_summary['focus_node_finish_ms_p95']:.1f}",
            "flat_focus_p95_ms": f"{flat_summary['focus_node_finish_ms_p95']:.1f}",
            "mean_ratio": f"{bootstrap['focus_node_finish_ms_mean']['ratio']:.3f}",
            "mean_ci_low": f"{bootstrap['focus_node_finish_ms_mean']['ci_low']:.3f}",
            "mean_ci_high": f"{bootstrap['focus_node_finish_ms_mean']['ci_high']:.3f}",
            "p95_ratio": f"{bootstrap['focus_node_finish_ms_p95']['ratio']:.3f}",
            "p95_ci_low": f"{bootstrap['focus_node_finish_ms_p95']['ci_low']:.3f}",
            "p95_ci_high": f"{bootstrap['focus_node_finish_ms_p95']['ci_high']:.3f}",
            "verdict": report["verdict"],
        })

    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if all(gates.values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
