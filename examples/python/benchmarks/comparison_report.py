#!/usr/bin/env python3
"""Summarize benchmark_e2e CSV output as Markdown.

The opt-level comparison path (the original report) is dependency-free and
keeps working without scipy. The arm-comparison path (APXM-on vs flat-HTTP
on the same CSV, paired by cell/seed) requires scipy for the Wilcoxon
signed-rank test; it is rendered only when the CSV carries an `arm` column.
"""

from __future__ import annotations

import argparse
import csv
import random
import statistics
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

try:
    from scipy.stats import wilcoxon as _scipy_wilcoxon  # type: ignore[import-not-found]
except ImportError:  # scipy not installed; arm-comparison section will note this
    _scipy_wilcoxon = None  # type: ignore[assignment]

_REPO_ROOT = Path(__file__).resolve().parents[3]
_APXM_PKG_DIR = str(_REPO_ROOT / "crates" / "compiler" / "apxm-frontend" / "python")
if _APXM_PKG_DIR not in sys.path:
    sys.path.insert(0, _APXM_PKG_DIR)

from apxm.contract import ArmName  # noqa: E402

DEFAULT_BOOTSTRAP_SAMPLES = 2000
DEFAULT_BOOTSTRAP_SEED = 42
LIST_SEPARATOR = ";"
MIN_SPEEDUP_ABS_MS = 1_000.0
MIN_SPEEDUP_RELATIVE = 0.05
MIN_TOKEN_DELTA = 500.0
MIN_TOKEN_REDUCTION = 0.05
MIN_CALL_DELTA = 1.0
RUNTIME_MODES = {"execute", "run-artifact"}
COMPILE_MODES = {"compile"}

ARM_COLUMN = "arm"
ARM_APXM_ON = ArmName.APXM_ON.value
ARM_FLAT_HTTP = ArmName.FLAT_HTTP.value

# Per-metric direction for the Wilcoxon alternative hypothesis. "less"
# means we hypothesize APXM-on values are smaller (latency-style metrics);
# "greater" means we hypothesize APXM-on values are larger (cache-reuse
# and pin-utilization metrics).
ARM_METRIC_SPECS: tuple[tuple[str, str], ...] = (
    ("batch_wall_ms", "less"),
    ("max_tenant_wall_ms", "less"),
    ("prefix_cache_hit_rate", "greater"),
    ("pinned_blocks_peak_max", "greater"),
    ("cached_input_tokens_sum", "greater"),
)

ARM_PAIR_KEY_FIELDS = ("cell_label", "opt_level", "iteration")


class CsvKey:
    ACTIVE_PASSES = "active_passes"
    ARTIFACT_PATH = "artifact_path"
    COMPILE_WALL_MS = "compile_wall_ms"
    COMPILER_PASSES_MS = "compiler_passes_ms"
    DIAGNOSTICS_PATH = "diagnostics_path"
    DSPY_FIRED_COUNT = "dspy_fired_count"
    FIRED_PASSES = "fired_passes"
    GRAPH_DURATION_MS = "graph_duration_ms"
    CRITICAL_MILESTONE_LAST_MS = "critical_milestone_last_ms"
    CRITICAL_MILESTONE_COUNT = "critical_milestone_count"
    OBSERVED_CRITICAL_PATH_MS = "observed_critical_path_ms"
    OBSERVED_CRITICAL_PATH_FINISH_MS = "observed_critical_path_finish_ms"
    QUEUE_WAIT_TOTAL_MS = "queue_wait_total_ms"
    QUEUE_WAIT_MEAN_MS = "queue_wait_mean_ms"
    QUEUE_WAIT_P95_MS = "queue_wait_p95_ms"
    CRITICAL_QUEUE_WAIT_TOTAL_MS = "critical_queue_wait_total_ms"
    LLM_CALL_COUNT = "llm_call_count"
    MODE = "mode"
    OPT_LEVEL = "opt_level"
    OPTIMIZATION_TARGET = "optimization_target"
    SUCCESS = "success"
    TOTAL_TOKENS = "total_tokens"
    CACHED_INPUT_TOKENS = "cached_input_tokens"
    REASONING_OUTPUT_TOKENS = "reasoning_output_tokens"
    TOTAL_OPS_ELIMINATED = "total_ops_eliminated"
    TOTAL_TOKENS_SAVED = "total_tokens_saved"
    WALL_MS = "wall_ms"


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv_path", type=Path, help="CSV emitted by benchmark_e2e.py")
    parser.add_argument(
        "--markdown-out",
        type=Path,
        default=None,
        help="Optional file path for the rendered Markdown report",
    )
    parser.add_argument(
        "--compare-opt-levels",
        nargs=2,
        type=int,
        default=None,
        metavar=("BASE", "TARGET"),
        help="Explicit pair of optimization levels to compare",
    )
    parser.add_argument(
        "--bootstrap-samples",
        type=int,
        default=DEFAULT_BOOTSTRAP_SAMPLES,
        help="Number of bootstrap resamples for the confidence interval",
    )
    return parser.parse_args()


def _read_rows(csv_path: Path) -> list[dict[str, str]]:
    with csv_path.open(newline="") as handle:
        return list(csv.DictReader(handle))


def _to_float(value: str) -> float | None:
    if not value:
        return None
    return float(value)


def _to_int(value: str) -> int | None:
    if not value:
        return None
    return int(value)


def _is_success(row: dict[str, str]) -> bool:
    return row.get(CsvKey.SUCCESS, "").lower() == "true"


def _pick_duration_ms(row: dict[str, str]) -> float | None:
    graph_duration = _to_float(row.get(CsvKey.GRAPH_DURATION_MS, ""))
    if graph_duration is not None:
        return graph_duration
    return _to_float(row.get(CsvKey.WALL_MS, ""))


def _pick_compile_wall_ms(row: dict[str, str]) -> float | None:
    compile_wall = _to_float(row.get(CsvKey.COMPILE_WALL_MS, ""))
    if compile_wall is not None:
        return compile_wall
    if row.get(CsvKey.MODE) == "compile":
        return _to_float(row.get(CsvKey.WALL_MS, ""))
    return None


def _pick_critical_milestone_ms(row: dict[str, str]) -> float | None:
    return _to_float(row.get(CsvKey.CRITICAL_MILESTONE_LAST_MS, ""))


def _bootstrap_mean_ci(
    values: list[float],
    samples: int,
    seed: int = DEFAULT_BOOTSTRAP_SEED,
) -> tuple[float, float] | None:
    if len(values) < 2:
        return None
    rng = random.Random(seed)
    means: list[float] = []
    for _ in range(samples):
        resample = [values[rng.randrange(len(values))] for _ in range(len(values))]
        means.append(statistics.fmean(resample))
    means.sort()
    lower_idx = int(0.025 * (len(means) - 1))
    upper_idx = int(0.975 * (len(means) - 1))
    return means[lower_idx], means[upper_idx]


def _bootstrap_speedup_ci(
    base_values: list[float],
    target_values: list[float],
    samples: int,
    seed: int = DEFAULT_BOOTSTRAP_SEED,
) -> tuple[float, float] | None:
    if len(base_values) < 2 or len(target_values) < 2:
        return None
    rng = random.Random(seed)
    ratios: list[float] = []
    for _ in range(samples):
        base_mean = statistics.fmean(
            [base_values[rng.randrange(len(base_values))] for _ in range(len(base_values))]
        )
        target_mean = statistics.fmean(
            [
                target_values[rng.randrange(len(target_values))]
                for _ in range(len(target_values))
            ]
        )
        if target_mean > 0:
            ratios.append(base_mean / target_mean)
    if len(ratios) < 2:
        return None
    ratios.sort()
    lower_idx = int(0.025 * (len(ratios) - 1))
    upper_idx = int(0.975 * (len(ratios) - 1))
    return ratios[lower_idx], ratios[upper_idx]


def _format_float(value: float | None, digits: int = 1) -> str:
    if value is None:
        return "-"
    return f"{value:.{digits}f}"


def _format_measurement(value: float | None, suffix: str = "", digits: int = 1) -> str:
    if value is None:
        return "unavailable"
    return f"{value:.{digits}f}{suffix}"


def _format_ci(bounds: tuple[float, float] | None, digits: int = 1) -> str:
    if bounds is None:
        return "-"
    return f"[{bounds[0]:.{digits}f}, {bounds[1]:.{digits}f}]"


def _pair_rows_by_arm(
    rows: list[dict[str, str]],
    key_fields: tuple[str, ...] = ARM_PAIR_KEY_FIELDS,
) -> dict[tuple, dict[str, dict[str, str]]]:
    """Group rows by the pairing key, splitting into the two arms.

    Returns a dict keyed by the pairing tuple where the value is itself a
    dict mapping arm name to the matching row. Pairs where one arm is
    missing or duplicated (>1 row per arm per key) are dropped.
    """
    by_key: dict[tuple, dict[str, list[dict[str, str]]]] = defaultdict(
        lambda: defaultdict(list)
    )
    for row in rows:
        arm = row.get(ARM_COLUMN, "").strip()
        if arm not in (ARM_APXM_ON, ARM_FLAT_HTTP):
            continue
        try:
            key = tuple(row[field] for field in key_fields)
        except KeyError:
            continue
        by_key[key][arm].append(row)
    paired: dict[tuple, dict[str, dict[str, str]]] = {}
    for key, arm_rows in by_key.items():
        apxm_rows = arm_rows.get(ARM_APXM_ON, [])
        flat_rows = arm_rows.get(ARM_FLAT_HTTP, [])
        if len(apxm_rows) == 1 and len(flat_rows) == 1:
            paired[key] = {ARM_APXM_ON: apxm_rows[0], ARM_FLAT_HTTP: flat_rows[0]}
    return paired


def _wilcoxon_p_value(
    apxm_values: list[float],
    flat_values: list[float],
    alternative: str,
) -> float | None:
    if _scipy_wilcoxon is None or len(apxm_values) != len(flat_values) or len(apxm_values) < 5:
        return None
    if all(a == b for a, b in zip(apxm_values, flat_values)):
        return None
    try:
        result = _scipy_wilcoxon(apxm_values, flat_values, alternative=alternative)
    except ValueError:
        return None
    return float(result.pvalue)


def _bootstrap_ratio_ci(
    apxm_values: list[float],
    flat_values: list[float],
    samples: int,
    seed: int = DEFAULT_BOOTSTRAP_SEED,
) -> tuple[float, float] | None:
    """CI on the ratio APXM_mean / flat_mean, paired-resampled by index."""
    if len(apxm_values) != len(flat_values) or len(apxm_values) < 2:
        return None
    rng = random.Random(seed)
    n = len(apxm_values)
    ratios: list[float] = []
    for _ in range(samples):
        indices = [rng.randrange(n) for _ in range(n)]
        apxm_mean = statistics.fmean(apxm_values[i] for i in indices)
        flat_mean = statistics.fmean(flat_values[i] for i in indices)
        if flat_mean > 0:
            ratios.append(apxm_mean / flat_mean)
    if len(ratios) < 2:
        return None
    ratios.sort()
    lower_idx = int(0.025 * (len(ratios) - 1))
    upper_idx = int(0.975 * (len(ratios) - 1))
    return ratios[lower_idx], ratios[upper_idx]


def _summarize_by_arm(
    rows: list[dict[str, str]],
    bootstrap_samples: int,
) -> dict[str, Any] | None:
    """Compute the paired arm comparison summary, or None if not applicable.

    Returns None when the CSV has no `arm` column, fewer than two pair
    keys, or no metric columns of interest.
    """
    if not rows or ARM_COLUMN not in rows[0]:
        return None
    paired = _pair_rows_by_arm(rows)
    if not paired:
        return None
    metrics: dict[str, dict[str, Any]] = {}
    for metric_name, alternative in ARM_METRIC_SPECS:
        if metric_name not in rows[0]:
            continue
        apxm_values: list[float] = []
        flat_values: list[float] = []
        for arm_pair in paired.values():
            apxm_v = _to_float(arm_pair[ARM_APXM_ON].get(metric_name, ""))
            flat_v = _to_float(arm_pair[ARM_FLAT_HTTP].get(metric_name, ""))
            if apxm_v is None or flat_v is None:
                continue
            apxm_values.append(apxm_v)
            flat_values.append(flat_v)
        if len(apxm_values) < 2:
            continue
        metrics[metric_name] = {
            "alternative": alternative,
            "paired_n": len(apxm_values),
            "apxm_median": statistics.median(apxm_values),
            "flat_median": statistics.median(flat_values),
            "wilcoxon_p": _wilcoxon_p_value(apxm_values, flat_values, alternative),
            "ratio_ci": _bootstrap_ratio_ci(apxm_values, flat_values, bootstrap_samples),
        }
    if not metrics:
        return None
    return {
        "pair_count": len(paired),
        "pair_key_fields": list(ARM_PAIR_KEY_FIELDS),
        "metrics": metrics,
        "scipy_available": _scipy_wilcoxon is not None,
    }


def _format_p_value(p: float | None) -> str:
    if p is None:
        return "-"
    if p < 1e-4:
        return f"{p:.2e}"
    return f"{p:.4f}"


def _render_arm_comparison(arm_summary: dict[str, Any]) -> str:
    lines: list[str] = []
    lines.append("\n## Arm comparison (APXM-on vs flat-HTTP)\n")
    lines.append(
        f"- Paired by `{', '.join(arm_summary['pair_key_fields'])}` "
        f"across {arm_summary['pair_count']} cells.\n"
    )
    if not arm_summary["scipy_available"]:
        lines.append(
            "- Wilcoxon column blank: scipy is not installed in this environment. "
            "Install with `uv pip install scipy` to enable the paired test.\n"
        )
    lines.append(
        "\n| metric | alt. | n | APXM median | flat median | Wilcoxon p | bootstrap 95% CI on ratio |\n"
    )
    lines.append("|---|---|---|---|---|---|---|\n")
    for metric_name, m in arm_summary["metrics"].items():
        ratio_ci = m["ratio_ci"]
        ci_str = (
            f"[{ratio_ci[0]:.4f}, {ratio_ci[1]:.4f}]" if ratio_ci is not None else "-"
        )
        lines.append(
            f"| `{metric_name}` | {m['alternative']} | {m['paired_n']} | "
            f"{m['apxm_median']:.4f} | {m['flat_median']:.4f} | "
            f"{_format_p_value(m['wilcoxon_p'])} | {ci_str} |\n"
        )
    return "".join(lines)


def _token_call_threshold_text() -> str:
    return (
        ">= "
        f"{MIN_TOKEN_DELTA:.0f} tokens and >= {MIN_TOKEN_REDUCTION * 100:.0f}% "
        f"token reduction, or >= {MIN_CALL_DELTA:.0f} fewer calls"
    )


def _token_call_claim_blocker(
    *,
    runtime_only: bool,
    has_replicates: bool,
    token_or_call_threshold_supported: bool,
) -> str:
    if not runtime_only:
        return "not supported; requires runtime rows, not compile-only rows"
    if not has_replicates and token_or_call_threshold_supported:
        return (
            "diagnostic only; token/call deltas clear the threshold, but the "
            "claim needs at least two successful runtime rows per optimization level"
        )
    if not has_replicates:
        return (
            "not supported; requires at least two successful runtime rows per "
            "optimization level plus " + _token_call_threshold_text()
        )
    return "not supported; requires " + _token_call_threshold_text()


def _cost_claim_blocker(
    *,
    runtime_only: bool,
    has_replicates: bool,
    token_or_call_threshold_supported: bool,
) -> str:
    if not runtime_only:
        return "not supported; requires runtime token/call deltas"
    if not has_replicates and token_or_call_threshold_supported:
        return (
            "diagnostic only; repeat the runtime rows, then apply explicit "
            "token prices or a hardware-cost model"
        )
    if not has_replicates:
        return (
            "not supported; requires repeated runtime rows, token/call "
            "thresholds, and explicit pricing or hardware-cost assumptions"
        )
    return (
        "not supported by token/call deltas; dollar claims also require "
        "explicit pricing or hardware-cost assumptions"
    )


def _summarize_by_opt(rows: list[dict[str, str]]) -> list[dict[str, Any]]:
    grouped: dict[int, list[dict[str, str]]] = defaultdict(list)
    for row in rows:
        grouped[int(row[CsvKey.OPT_LEVEL])].append(row)

    summaries: list[dict[str, Any]] = []
    for opt_level in sorted(grouped):
        group_rows = grouped[opt_level]
        success_rows = [row for row in group_rows if _is_success(row)]
        durations = [
            duration
            for row in success_rows
            if (duration := _pick_duration_ms(row)) is not None
        ]
        critical_milestones = [
            duration
            for row in success_rows
            if (duration := _pick_critical_milestone_ms(row)) is not None
        ]
        observed_critical_paths = [
            duration
            for row in success_rows
            if (duration := _to_float(row.get(CsvKey.OBSERVED_CRITICAL_PATH_MS, ""))) is not None
        ]
        observed_critical_finishes = [
            duration
            for row in success_rows
            if (duration := _to_float(row.get(CsvKey.OBSERVED_CRITICAL_PATH_FINISH_MS, ""))) is not None
        ]
        queue_wait_totals = [
            duration
            for row in success_rows
            if (duration := _to_float(row.get(CsvKey.QUEUE_WAIT_TOTAL_MS, ""))) is not None
        ]
        queue_wait_means = [
            duration
            for row in success_rows
            if (duration := _to_float(row.get(CsvKey.QUEUE_WAIT_MEAN_MS, ""))) is not None
        ]
        queue_wait_p95s = [
            duration
            for row in success_rows
            if (duration := _to_float(row.get(CsvKey.QUEUE_WAIT_P95_MS, ""))) is not None
        ]
        critical_queue_wait_totals = [
            duration
            for row in success_rows
            if (duration := _to_float(row.get(CsvKey.CRITICAL_QUEUE_WAIT_TOTAL_MS, ""))) is not None
        ]
        critical_milestone_counts = [
            count
            for row in success_rows
            if (count := _to_int(row.get(CsvKey.CRITICAL_MILESTONE_COUNT, ""))) is not None
        ]
        token_totals = [
            total
            for row in success_rows
            if (total := _to_int(row.get(CsvKey.TOTAL_TOKENS, ""))) is not None
        ]
        cached_input_totals = [
            total
            for row in success_rows
            if (total := _to_int(row.get(CsvKey.CACHED_INPUT_TOKENS, ""))) is not None
        ]
        reasoning_output_totals = [
            total
            for row in success_rows
            if (total := _to_int(row.get(CsvKey.REASONING_OUTPUT_TOKENS, ""))) is not None
        ]
        llm_calls = [
            count
            for row in success_rows
            if (count := _to_int(row.get(CsvKey.LLM_CALL_COUNT, ""))) is not None
        ]
        compiler_pass_ms = [
            duration
            for row in success_rows
            if (duration := _to_float(row.get(CsvKey.COMPILER_PASSES_MS, ""))) is not None
        ]
        compile_wall_ms_values = [
            duration
            for row in success_rows
            if (duration := _pick_compile_wall_ms(row)) is not None
        ]
        total_ops_eliminated = [
            count
            for row in success_rows
            if (count := _to_int(row.get(CsvKey.TOTAL_OPS_ELIMINATED, ""))) is not None
        ]
        total_tokens_saved = [
            count
            for row in success_rows
            if (count := _to_int(row.get(CsvKey.TOTAL_TOKENS_SAVED, ""))) is not None
        ]
        dspy_fired_counts = [
            count
            for row in success_rows
            if (count := _to_int(row.get(CsvKey.DSPY_FIRED_COUNT, ""))) is not None
        ]

        summaries.append(
            {
                "opt_level": opt_level,
                "total_runs": len(group_rows),
                "successful_runs": len(success_rows),
                "success_rate": len(success_rows) / len(group_rows) if group_rows else 0.0,
                "durations": durations,
                "critical_milestones": critical_milestones,
                "observed_critical_paths": observed_critical_paths,
                "observed_critical_finishes": observed_critical_finishes,
                "mean_duration_ms": statistics.fmean(durations) if durations else None,
                "median_duration_ms": statistics.median(durations) if durations else None,
                "mean_critical_milestone_ms": statistics.fmean(critical_milestones)
                if critical_milestones
                else None,
                "median_critical_milestone_ms": statistics.median(critical_milestones)
                if critical_milestones
                else None,
                "mean_critical_milestone_count": statistics.fmean(critical_milestone_counts)
                if critical_milestone_counts
                else None,
                "mean_observed_critical_path_ms": statistics.fmean(observed_critical_paths)
                if observed_critical_paths
                else None,
                "mean_observed_critical_finish_ms": statistics.fmean(observed_critical_finishes)
                if observed_critical_finishes
                else None,
                "mean_queue_wait_total_ms": statistics.fmean(queue_wait_totals)
                if queue_wait_totals
                else None,
                "mean_queue_wait_ms": statistics.fmean(queue_wait_means)
                if queue_wait_means
                else None,
                "mean_queue_wait_p95_ms": statistics.fmean(queue_wait_p95s)
                if queue_wait_p95s
                else None,
                "mean_critical_queue_wait_total_ms": statistics.fmean(critical_queue_wait_totals)
                if critical_queue_wait_totals
                else None,
                "mean_total_tokens": statistics.fmean(token_totals) if token_totals else None,
                "mean_cached_input_tokens": statistics.fmean(cached_input_totals)
                if cached_input_totals
                else None,
                "mean_reasoning_output_tokens": statistics.fmean(reasoning_output_totals)
                if reasoning_output_totals
                else None,
                "mean_llm_calls": statistics.fmean(llm_calls) if llm_calls else None,
                "mean_compiler_passes_ms": statistics.fmean(compiler_pass_ms)
                if compiler_pass_ms
                else None,
                "mean_compile_wall_ms": statistics.fmean(compile_wall_ms_values)
                if compile_wall_ms_values
                else None,
                "mean_total_ops_eliminated": statistics.fmean(total_ops_eliminated)
                if total_ops_eliminated
                else None,
                "mean_total_tokens_saved": statistics.fmean(total_tokens_saved)
                if total_tokens_saved
                else None,
                "mean_dspy_fired_count": statistics.fmean(dspy_fired_counts)
                if dspy_fired_counts
                else None,
                "fired_passes": sorted(
                    {
                        part
                        for row in success_rows
                        for part in row.get(CsvKey.FIRED_PASSES, "").split(LIST_SEPARATOR)
                        if part
                    }
                ),
                "active_passes": sorted(
                    {
                        part
                        for row in success_rows
                        for part in row.get(CsvKey.ACTIVE_PASSES, "").split(LIST_SEPARATOR)
                        if part
                    }
                ),
                "diagnostics_paths": [
                    row[CsvKey.DIAGNOSTICS_PATH]
                    for row in success_rows
                    if row.get(CsvKey.DIAGNOSTICS_PATH)
                ],
                "artifact_paths": sorted(
                    {
                        row.get(CsvKey.ARTIFACT_PATH, "")
                        for row in success_rows
                        if row.get(CsvKey.ARTIFACT_PATH)
                    }
                ),
            }
        )
    return summaries


def _render_markdown(
    csv_path: Path,
    rows: list[dict[str, str]],
    summaries: list[dict[str, Any]],
    compare_opt_levels: tuple[int, int] | None,
    bootstrap_samples: int,
) -> str:
    modes = sorted({row.get(CsvKey.MODE, "") or "unknown" for row in rows})
    targets = sorted({row.get(CsvKey.OPTIMIZATION_TARGET, "") or "unknown" for row in rows})
    mode_set = set(modes)
    runtime_only = mode_set.issubset(RUNTIME_MODES)
    compile_only = mode_set.issubset(COMPILE_MODES)

    lines: list[str] = []
    lines.append("# APXM graph benchmark report")
    lines.append("")
    lines.append(f"- Source CSV: `{csv_path}`")
    lines.append(f"- Rows: {len(rows)}")
    lines.append(f"- Modes: {', '.join(modes)}")
    lines.append(f"- Optimization targets: {', '.join(targets)}")
    if not runtime_only:
        lines.append(
            "- Claim status: non-runtime rows validate compiler behavior; "
            "they do not support runtime speed, token, or cost claims."
        )
    lines.append("")
    lines.append("## Per-opt-level summary")
    lines.append("")
    lines.append(
        "| opt | successful / total | success % | mean runtime ms | median runtime ms | "
        "mean critical milestone ms | observed critical path ms | queue wait total ms | "
        "queue wait p95 ms | mean compile ms | mean total tokens | mean cached input | "
        "mean reasoning output | mean LLM calls |"
    )
    lines.append("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")

    for summary in summaries:
        ci = _bootstrap_mean_ci(summary["durations"], bootstrap_samples)
        mean_with_ci = _format_float(summary["mean_duration_ms"])
        if ci is not None:
            mean_with_ci = f"{mean_with_ci} {_format_ci(ci)}"
        lines.append(
            f"| O{summary['opt_level']} | "
            f"{summary['successful_runs']} / {summary['total_runs']} | "
            f"{summary['success_rate'] * 100:.1f} | "
            f"{mean_with_ci} | "
            f"{_format_float(summary['median_duration_ms'])} | "
            f"{_format_float(summary['mean_critical_milestone_ms'])} | "
            f"{_format_float(summary['mean_observed_critical_path_ms'])} | "
            f"{_format_float(summary['mean_queue_wait_total_ms'])} | "
            f"{_format_float(summary['mean_queue_wait_p95_ms'])} | "
            f"{_format_float(summary['mean_compile_wall_ms'])} | "
            f"{_format_float(summary['mean_total_tokens'])} | "
            f"{_format_float(summary['mean_cached_input_tokens'])} | "
            f"{_format_float(summary['mean_reasoning_output_tokens'])} | "
            f"{_format_float(summary['mean_llm_calls'])} |"
        )

    if any(summary["artifact_paths"] for summary in summaries):
        lines.append("")
        lines.append(
            "Runtime rows are repeated `.apxmobj` executions. `mean compile ms` is the "
            "one-time precompile wall time for the artifact at that optimization level, "
            "not time paid by every runtime sample."
        )

    if any(summary["diagnostics_paths"] for summary in summaries):
        lines.append("")
        lines.append("## Compiler diagnostics")
        lines.append("")
        if compile_only:
            lines.append(
                "These rows validate compiler behavior only. They show which passes "
                "ran or fired, but they do not support runtime speed, token, or cost claims."
            )
        elif "run-artifact" in mode_set:
            lines.append(
                "These diagnostics were captured from the precompile step attached to "
                "each artifact run row, so runtime metrics are separated from compiler "
                "overhead while still showing which passes fired."
            )
        else:
            lines.append(
                "These diagnostics were captured from the compile step attached to each "
                "execute row, so runtime metrics can be tied to the passes that actually fired."
            )
        lines.append("")
        lines.append(
            "| opt | mean compiler passes ms | fired passes | active passes | "
            "mean ops eliminated | mean tokens saved | mean dspy fired | diagnostics |"
        )
        lines.append("|---:|---:|---|---|---:|---:|---:|---:|")
        for summary in summaries:
            lines.append(
                f"| O{summary['opt_level']} | "
                f"{_format_float(summary['mean_compiler_passes_ms'])} | "
                f"{', '.join(summary['fired_passes']) or '-'} | "
                f"{', '.join(summary['active_passes']) or '-'} | "
                f"{_format_float(summary['mean_total_ops_eliminated'])} | "
                f"{_format_float(summary['mean_total_tokens_saved'])} | "
                f"{_format_float(summary['mean_dspy_fired_count'])} | "
                f"{len(summary['diagnostics_paths'])} |"
            )

    if runtime_only and len(summaries) >= 2:
        if compare_opt_levels is None:
            compare_opt_levels = (summaries[0]["opt_level"], summaries[-1]["opt_level"])

        summary_by_opt = {summary["opt_level"]: summary for summary in summaries}
        base = summary_by_opt.get(compare_opt_levels[0])
        target = summary_by_opt.get(compare_opt_levels[1])

        if base is not None and target is not None:
            base_mean = base["mean_duration_ms"]
            target_mean = target["mean_duration_ms"]
            speedup = None
            if base_mean and target_mean:
                speedup = base_mean / target_mean if target_mean > 0 else None
            speedup_ci = _bootstrap_speedup_ci(
                base["durations"],
                target["durations"],
                bootstrap_samples,
            )
            critical_base_mean = base["mean_critical_milestone_ms"]
            critical_target_mean = target["mean_critical_milestone_ms"]
            critical_speedup = None
            if critical_base_mean and critical_target_mean:
                critical_speedup = (
                    critical_base_mean / critical_target_mean
                    if critical_target_mean > 0
                    else None
                )
            critical_speedup_ci = _bootstrap_speedup_ci(
                base["critical_milestones"],
                target["critical_milestones"],
                bootstrap_samples,
            )
            observed_cp_base_mean = base["mean_observed_critical_path_ms"]
            observed_cp_target_mean = target["mean_observed_critical_path_ms"]
            observed_cp_speedup = None
            if observed_cp_base_mean and observed_cp_target_mean:
                observed_cp_speedup = (
                    observed_cp_base_mean / observed_cp_target_mean
                    if observed_cp_target_mean > 0
                    else None
                )
            observed_cp_speedup_ci = _bootstrap_speedup_ci(
                base["observed_critical_paths"],
                target["observed_critical_paths"],
                bootstrap_samples,
            )
            observed_cp_delta = None
            observed_cp_reduction = None
            if observed_cp_base_mean is not None and observed_cp_target_mean is not None:
                observed_cp_delta = observed_cp_base_mean - observed_cp_target_mean
                if observed_cp_base_mean > 0:
                    observed_cp_reduction = observed_cp_delta / observed_cp_base_mean
            observed_finish_base_mean = base["mean_observed_critical_finish_ms"]
            observed_finish_target_mean = target["mean_observed_critical_finish_ms"]
            observed_finish_delta = None
            if observed_finish_base_mean is not None and observed_finish_target_mean is not None:
                observed_finish_delta = observed_finish_base_mean - observed_finish_target_mean
            queue_wait_base_mean = base["mean_queue_wait_total_ms"]
            queue_wait_target_mean = target["mean_queue_wait_total_ms"]
            queue_wait_delta = None
            queue_wait_reduction = None
            if queue_wait_base_mean is not None and queue_wait_target_mean is not None:
                queue_wait_delta = queue_wait_base_mean - queue_wait_target_mean
                if queue_wait_base_mean > 0:
                    queue_wait_reduction = queue_wait_delta / queue_wait_base_mean
            critical_delta = None
            critical_reduction = None
            if critical_base_mean is not None and critical_target_mean is not None:
                critical_delta = critical_base_mean - critical_target_mean
                if critical_base_mean > 0:
                    critical_reduction = critical_delta / critical_base_mean
            token_delta = None
            token_reduction = None
            if base["mean_total_tokens"] is not None and target["mean_total_tokens"] is not None:
                token_delta = base["mean_total_tokens"] - target["mean_total_tokens"]
                if base["mean_total_tokens"] > 0:
                    token_reduction = token_delta / base["mean_total_tokens"]
            call_delta = None
            call_reduction = None
            if base["mean_llm_calls"] is not None and target["mean_llm_calls"] is not None:
                call_delta = base["mean_llm_calls"] - target["mean_llm_calls"]
                if base["mean_llm_calls"] > 0:
                    call_reduction = call_delta / base["mean_llm_calls"]
            cached_delta = None
            if (
                base["mean_cached_input_tokens"] is not None
                and target["mean_cached_input_tokens"] is not None
            ):
                cached_delta = target["mean_cached_input_tokens"] - base[
                    "mean_cached_input_tokens"
                ]
            reasoning_delta = None
            if (
                base["mean_reasoning_output_tokens"] is not None
                and target["mean_reasoning_output_tokens"] is not None
            ):
                reasoning_delta = target["mean_reasoning_output_tokens"] - base[
                    "mean_reasoning_output_tokens"
                ]
            has_replicates = len(base["durations"]) >= 2 and len(target["durations"]) >= 2
            duration_delta = None
            duration_reduction = None
            if base_mean is not None and target_mean is not None:
                duration_delta = base_mean - target_mean
                if base_mean > 0:
                    duration_reduction = duration_delta / base_mean
            base_compile = base["mean_compile_wall_ms"]
            target_compile = target["mean_compile_wall_ms"]
            base_first_run = (
                base_compile + base_mean
                if base_compile is not None and base_mean is not None
                else None
            )
            target_first_run = (
                target_compile + target_mean
                if target_compile is not None and target_mean is not None
                else None
            )
            first_run_delta = (
                base_first_run - target_first_run
                if base_first_run is not None and target_first_run is not None
                else None
            )
            token_reduction_label = (
                "-"
                if token_reduction is None
                else f"{token_reduction * 100:.1f}%"
            )
            call_reduction_label = (
                "-"
                if call_reduction is None
                else f"{call_reduction * 100:.1f}%"
            )
            token_savings_supported = (
                token_delta is not None
                and token_reduction is not None
                and token_delta >= MIN_TOKEN_DELTA
                and token_reduction >= MIN_TOKEN_REDUCTION
            )
            call_savings_supported = call_delta is not None and call_delta >= MIN_CALL_DELTA
            token_or_call_threshold_supported = (
                token_savings_supported or call_savings_supported
            )
            savings_supported = runtime_only and has_replicates and (
                token_or_call_threshold_supported
            )
            speed_supported = (
                runtime_only
                and has_replicates
                and duration_delta is not None
                and duration_delta >= MIN_SPEEDUP_ABS_MS
                and duration_reduction is not None
                and duration_reduction >= MIN_SPEEDUP_RELATIVE
                and speedup is not None
                and speedup > 1.0
                and speedup_ci is not None
                and speedup_ci[0] > 1.0
            )
            critical_speed_supported = (
                runtime_only
                and len(base["critical_milestones"]) >= 2
                and len(target["critical_milestones"]) >= 2
                and critical_delta is not None
                and critical_delta >= MIN_SPEEDUP_ABS_MS
                and critical_reduction is not None
                and critical_reduction >= MIN_SPEEDUP_RELATIVE
                and critical_speedup is not None
                and critical_speedup > 1.0
                and critical_speedup_ci is not None
                and critical_speedup_ci[0] > 1.0
            )
            observed_cp_supported = (
                runtime_only
                and len(base["observed_critical_paths"]) >= 2
                and len(target["observed_critical_paths"]) >= 2
                and observed_cp_delta is not None
                and observed_cp_delta >= MIN_SPEEDUP_ABS_MS
                and observed_cp_reduction is not None
                and observed_cp_reduction >= MIN_SPEEDUP_RELATIVE
                and observed_cp_speedup is not None
                and observed_cp_speedup > 1.0
                and observed_cp_speedup_ci is not None
                and observed_cp_speedup_ci[0] > 1.0
            )

            lines.append("")
            lines.append("## Comparison")
            lines.append("")
            lines.append(
                f"- Baseline: O{base['opt_level']}"
            )
            lines.append(
                f"- Target: O{target['opt_level']}"
            )
            lines.append(
                f"- Mean-duration speedup: {_format_measurement(speedup, suffix='x', digits=2)}"
            )
            lines.append(
                f"- Mean-duration reduction: "
                f"{_format_measurement(duration_delta, suffix=' ms')} "
                f"({_format_measurement(None if duration_delta is None else duration_delta / 1000.0, suffix=' s', digits=2)}; "
                f"{'-' if duration_reduction is None else f'{duration_reduction * 100:.1f}%'})"
            )
            if base_compile is not None or target_compile is not None:
                lines.append(
                    f"- One-time compile wall: O{base['opt_level']} "
                    f"{_format_measurement(base_compile, suffix=' ms')}; "
                    f"O{target['opt_level']} "
                    f"{_format_measurement(target_compile, suffix=' ms')}"
                )
                lines.append(
                    f"- First-run compile+runtime estimate: O{base['opt_level']} "
                    f"{_format_measurement(base_first_run, suffix=' ms')}; "
                    f"O{target['opt_level']} "
                    f"{_format_measurement(target_first_run, suffix=' ms')}; "
                    f"delta {_format_measurement(first_run_delta, suffix=' ms')}"
                )
            lines.append(
                f"- Bootstrap 95% CI for speedup: {_format_ci(speedup_ci, digits=2)}"
            )
            if critical_base_mean is not None or critical_target_mean is not None:
                lines.append(
                    f"- Mean critical-milestone speedup: "
                    f"{_format_measurement(critical_speedup, suffix='x', digits=2)}"
                )
                lines.append(
                    f"- Mean critical-milestone reduction: "
                    f"{_format_measurement(critical_delta, suffix=' ms')} "
                    f"({_format_measurement(None if critical_delta is None else critical_delta / 1000.0, suffix=' s', digits=2)}; "
                    f"{'-' if critical_reduction is None else f'{critical_reduction * 100:.1f}%'})"
                )
                lines.append(
                    f"- Bootstrap 95% CI for critical-milestone speedup: "
                    f"{_format_ci(critical_speedup_ci, digits=2)}"
                )
            if observed_cp_base_mean is not None or observed_cp_target_mean is not None:
                lines.append(
                    f"- Observed critical-path speedup: "
                    f"{_format_measurement(observed_cp_speedup, suffix='x', digits=2)}"
                )
                lines.append(
                    f"- Observed critical-path reduction: "
                    f"{_format_measurement(observed_cp_delta, suffix=' ms')} "
                    f"({_format_measurement(None if observed_cp_delta is None else observed_cp_delta / 1000.0, suffix=' s', digits=2)}; "
                    f"{'-' if observed_cp_reduction is None else f'{observed_cp_reduction * 100:.1f}%'})"
                )
                lines.append(
                    f"- Bootstrap 95% CI for observed critical-path speedup: "
                    f"{_format_ci(observed_cp_speedup_ci, digits=2)}"
                )
            if observed_finish_delta is not None:
                lines.append(
                    f"- Observed critical-path finish reduction: "
                    f"{_format_measurement(observed_finish_delta, suffix=' ms')}"
                )
            if queue_wait_delta is not None:
                queue_wait_reduction_label = (
                    "-"
                    if queue_wait_reduction is None
                    else f"{queue_wait_reduction * 100:.1f}%"
                )
                lines.append(
                    f"- Total scheduler queue-wait reduction: "
                    f"{_format_measurement(queue_wait_delta, suffix=' ms')} "
                    f"({queue_wait_reduction_label})"
                )
            lines.append(
                "- Claim threshold: speedup requires >= "
                f"{MIN_SPEEDUP_ABS_MS / 1000:.1f}s absolute reduction, "
                f">= {MIN_SPEEDUP_RELATIVE * 100:.0f}% relative reduction, "
                "and bootstrap lower bound > 1.0."
            )
            lines.append(
                f"- Success-rate delta: "
                f"{(target['success_rate'] - base['success_rate']) * 100:.1f} percentage points"
            )
            lines.append(
                f"- Mean-token reduction: {_format_measurement(token_delta, suffix=' tokens')} "
                f"({token_reduction_label})"
            )
            lines.append(
                f"- Mean-call reduction: {_format_measurement(call_delta, suffix=' calls')} "
                f"({call_reduction_label})"
            )
            lines.append(
                f"- Mean cached-input increase: "
                f"{_format_measurement(cached_delta, suffix=' tokens')}"
            )
            lines.append(
                f"- Mean reasoning-output increase: "
                f"{_format_measurement(reasoning_delta, suffix=' tokens')}"
            )
            lines.append(
                "- Speedup claim: "
                + (
                    "supported by mean runtime duration"
                    if speed_supported
                    else "not supported; use repeated live rows before claiming speedup"
                )
            )
            if critical_base_mean is not None or critical_target_mean is not None:
                lines.append(
                    "- Critical-milestone latency claim: "
                    + (
                        "supported by measured critical-result completion time"
                        if critical_speed_supported
                        else (
                            "not supported; requires repeated milestone rows, >= "
                            f"{MIN_SPEEDUP_ABS_MS / 1000:.1f}s absolute reduction, "
                            f">= {MIN_SPEEDUP_RELATIVE * 100:.0f}% relative reduction, "
                            "and bootstrap lower bound > 1.0"
                        )
                    )
                )
            if observed_cp_base_mean is not None or observed_cp_target_mean is not None:
                lines.append(
                    "- Observed critical-path claim: "
                    + (
                        "supported by runtime node durations and DAG edges"
                        if observed_cp_supported
                        else (
                            "not supported; requires repeated observed-graph rows, >= "
                            f"{MIN_SPEEDUP_ABS_MS / 1000:.1f}s absolute reduction, "
                            f">= {MIN_SPEEDUP_RELATIVE * 100:.0f}% relative reduction, "
                            "and bootstrap lower bound > 1.0"
                        )
                    )
                )
            lines.append(
                "- Token/call reduction claim: "
                + (
                    "supported by measured token or call reduction"
                    if savings_supported
                    else _token_call_claim_blocker(
                        runtime_only=runtime_only,
                        has_replicates=has_replicates,
                        token_or_call_threshold_supported=token_or_call_threshold_supported,
                    )
                )
            )
            lines.append(
                "- Cost-reduction claim: "
                + (
                    "allowed only as a priced or hardware-cost model using the measured deltas"
                    if savings_supported
                    else _cost_claim_blocker(
                        runtime_only=runtime_only,
                        has_replicates=has_replicates,
                        token_or_call_threshold_supported=token_or_call_threshold_supported,
                    )
                )
            )

    return "\n".join(lines) + "\n"


def main() -> int:
    args = _parse_args()
    csv_path = args.csv_path.resolve()
    if not csv_path.exists():
        raise SystemExit(f"error: CSV not found: {csv_path}")

    rows = _read_rows(csv_path)
    if not rows:
        raise SystemExit(f"error: CSV is empty: {csv_path}")

    summaries = _summarize_by_opt(rows)
    compare_opt_levels = None
    if args.compare_opt_levels is not None:
        compare_opt_levels = (args.compare_opt_levels[0], args.compare_opt_levels[1])

    markdown = _render_markdown(
        csv_path=csv_path,
        rows=rows,
        summaries=summaries,
        compare_opt_levels=compare_opt_levels,
        bootstrap_samples=args.bootstrap_samples,
    )

    arm_summary = _summarize_by_arm(rows, args.bootstrap_samples)
    if arm_summary is not None:
        markdown = markdown + _render_arm_comparison(arm_summary)

    if args.markdown_out is not None:
        args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_out.write_text(markdown)

    sys.stdout.write(markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
