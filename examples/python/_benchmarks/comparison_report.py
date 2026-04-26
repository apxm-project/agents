#!/usr/bin/env python3
"""Summarize benchmark_e2e CSV output as Markdown.

The report intentionally stays dependency-free so it works in a fresh checkout.
It focuses on the metrics the local harness can reliably collect:

- success rate
- wall-clock timing
- graph duration when emitted by the session writer
- token/call totals when available
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


DEFAULT_BOOTSTRAP_SAMPLES = 2000
DEFAULT_BOOTSTRAP_SEED = 42
LIST_SEPARATOR = ";"


class CsvKey:
    ACTIVE_PASSES = "active_passes"
    COMPILER_PASSES_MS = "compiler_passes_ms"
    DIAGNOSTICS_PATH = "diagnostics_path"
    DSPY_FIRED_COUNT = "dspy_fired_count"
    FIRED_PASSES = "fired_passes"
    GRAPH_DURATION_MS = "graph_duration_ms"
    LLM_CALL_COUNT = "llm_call_count"
    MODE = "mode"
    OPT_LEVEL = "opt_level"
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
                "mean_duration_ms": statistics.fmean(durations) if durations else None,
                "median_duration_ms": statistics.median(durations) if durations else None,
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
    execute_only = modes == ["execute"]
    compile_only = modes == ["compile"]

    lines: list[str] = []
    lines.append("# APXM graph benchmark report")
    lines.append("")
    lines.append(f"- Source CSV: `{csv_path}`")
    lines.append(f"- Rows: {len(rows)}")
    lines.append(f"- Modes: {', '.join(modes)}")
    if not execute_only:
        lines.append(
            "- Claim status: compile-only rows validate compiler behavior; "
            "they do not support runtime speed, token, or cost claims."
        )
    lines.append("")
    lines.append("## Per-opt-level summary")
    lines.append("")
    lines.append(
        "| opt | successful / total | success % | mean duration ms | median duration ms | "
        "mean total tokens | mean cached input | mean reasoning output | mean LLM calls |"
    )
    lines.append("|---:|---:|---:|---:|---:|---:|---:|---:|---:|")

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
            f"{_format_float(summary['mean_total_tokens'])} | "
            f"{_format_float(summary['mean_cached_input_tokens'])} | "
            f"{_format_float(summary['mean_reasoning_output_tokens'])} | "
            f"{_format_float(summary['mean_llm_calls'])} |"
        )

    if compile_only and any(summary["diagnostics_paths"] for summary in summaries):
        lines.append("")
        lines.append("## Compiler diagnostics")
        lines.append("")
        lines.append(
            "These rows validate compiler behavior only. They show which passes "
            "ran or fired, but they do not support runtime speed, token, or cost claims."
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

    if execute_only and len(summaries) >= 2:
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
            savings_supported = execute_only and has_replicates and (
                (token_reduction is not None and token_reduction > 0)
                or (call_reduction is not None and call_reduction > 0)
            )
            speed_supported = (
                execute_only
                and has_replicates
                and speedup is not None
                and speedup > 1.0
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
                f"- Bootstrap 95% CI for speedup: {_format_ci(speedup_ci, digits=2)}"
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
            lines.append(
                "- Token/call reduction claim: "
                + (
                    "supported by measured token or call reduction"
                    if savings_supported
                    else "not supported by repeated live token/call deltas"
                )
            )
            lines.append(
                "- Cost-reduction claim: "
                + (
                    "allowed only as a priced or hardware-cost model using the measured deltas"
                    if savings_supported
                    else "not supported by token/call deltas"
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

    if args.markdown_out is not None:
        args.markdown_out.parent.mkdir(parents=True, exist_ok=True)
        args.markdown_out.write_text(markdown)

    sys.stdout.write(markdown)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
