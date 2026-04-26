#!/usr/bin/env python3
"""Run repeated APXM graph benchmarks and emit a CSV summary.

This harness stays deliberately small:
- it shells out to `dekk apxm execute` for real runs
- it supports `--compile-only` for backend-free validation
- it records wall-clock timing plus session-derived metrics when available

Usage:
  python3 examples/python/_benchmarks/benchmark_e2e.py --compile-only
  python3 examples/python/_benchmarks/benchmark_e2e.py --iterations 5
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from enum import StrEnum
from pathlib import Path
from typing import Any


def _find_repo_root(start: Path) -> Path:
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / "Cargo.toml").is_file() and (candidate / "crates").is_dir():
            return candidate
    return Path.cwd().resolve()


REPO_ROOT = _find_repo_root(Path(__file__))
BENCHMARK_DIR = Path(__file__).resolve().parent
DEFAULT_GRAPH = BENCHMARK_DIR / "demo_code_critique.py"
DEFAULT_RESULTS_DIR = BENCHMARK_DIR / "results"
DEFAULT_OUTPUT = DEFAULT_RESULTS_DIR / "demo_code_critique_benchmark.csv"
DEFAULT_SESSION_BASE = DEFAULT_RESULTS_DIR / "sessions"

DEKK = "dekk"
APXM = "apxm"
CMD_EXECUTE = "execute"
CMD_COMPILE = "compile"
FLAG_OUTPUT = "-o"
FLAG_OPT_LEVEL = "-O"
FLAG_EMIT_SESSION = "--emit-session"
FLAG_TRACE = "--trace"
ENV_APXM_CONFIG = "APXM_CONFIG"

FILE_MANIFEST = "manifest.json"
FILE_RESULTS = "results.json"
FILE_METRICS = "metrics.json"
RUN_SUMMARY_PREFIX = "benchmark_e2e run"
PENDING_VALUE = "pending"
STATUS_SUCCEEDED = "succeeded"
STATUS_FAILED = "failed"


class MetricsKey(StrEnum):
    SCHEMA_VERSION = "schema_version"
    RUNTIME = "runtime"
    BACKENDS = "backends"
    GRAPHS = "graphs"
    TOKEN_ACCOUNTING = "token_accounting"
    TOTAL = "total"
    INPUT_TOKENS = "input_tokens"
    OUTPUT_TOKENS = "output_tokens"
    TOTAL_TOKENS = "total_tokens"
    CACHED_INPUT_TOKENS = "cached_input_tokens"
    REASONING_OUTPUT_TOKENS = "reasoning_output_tokens"
    CALL_COUNT = "call_count"
    PINNED_BLOCKS = "pinned_blocks"
    PINNED_HANDLES = "pinned_handles"
    CRITICAL_PATH_LENGTH = "critical_path_length"


SCHEMA_VERSION_V2 = 2

KEY_DURATION_MS = "duration_ms"
KEY_FINAL_OUTPUT = "final_output"


def _backend_graphs(metrics: dict[str, Any]) -> list[dict[str, Any]]:
    backends = metrics.get(MetricsKey.BACKENDS, {})
    if not isinstance(backends, dict):
        return []

    graphs = backends.get(MetricsKey.GRAPHS, [])
    if not isinstance(graphs, list):
        return []
    return [graph for graph in graphs if isinstance(graph, dict)]

CSV_FIELDS = [
    "timestamp_utc",
    "graph",
    "mode",
    "backend_label",
    "opt_level",
    "run_index",
    "trial_id",
    "run_order",
    "success",
    "wall_ms",
    "graph_duration_ms",
    "llm_call_count",
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "cached_input_tokens",
    "reasoning_output_tokens",
    "pinned_blocks",
    "pinned_handles",
    "critical_path_length",
    "session_dir",
    "output_excerpt",
    "stderr_excerpt",
]


@dataclass
class SessionSummary:
    graph_duration_ms: int | None
    llm_call_count: int | None
    input_tokens: int | None
    output_tokens: int | None
    total_tokens: int | None
    cached_input_tokens: int | None
    reasoning_output_tokens: int | None
    pinned_blocks: int | None
    pinned_handles: int | None
    critical_path_length: int | None
    final_output: str

    @classmethod
    def from_session(cls, session_root: Path) -> SessionSummary:
        graph_duration_ms: int | None = None
        final_output = ""
        llm_call_count: int | None = None
        input_tokens: int | None = None
        output_tokens: int | None = None
        total_tokens: int | None = None
        cached_input_tokens: int | None = None
        reasoning_output_tokens: int | None = None
        pinned_blocks: int | None = None
        pinned_handles: int | None = None
        critical_path_length: int | None = None

        manifest_path = session_root / FILE_MANIFEST
        if manifest_path.exists():
            manifest = json.loads(manifest_path.read_text())
            duration = manifest.get(KEY_DURATION_MS)
            if isinstance(duration, int):
                graph_duration_ms = duration

        results_path = session_root / FILE_RESULTS
        if results_path.exists():
            results = json.loads(results_path.read_text())
            out = results.get(KEY_FINAL_OUTPUT)
            if isinstance(out, str):
                final_output = out

        metrics_path = session_root / FILE_METRICS
        if metrics_path.exists():
            metrics = json.loads(metrics_path.read_text())
            version = metrics.get(MetricsKey.SCHEMA_VERSION)
            if version != SCHEMA_VERSION_V2:
                raise ValueError(
                    f"Expected metrics schema_version={SCHEMA_VERSION_V2}, got {version}"
                )

            total = (
                metrics.get(MetricsKey.RUNTIME, {})
                .get(MetricsKey.TOKEN_ACCOUNTING, {})
                .get(MetricsKey.TOTAL, {})
            )
            v = total.get(MetricsKey.CALL_COUNT)
            if isinstance(v, int):
                llm_call_count = v
            v = total.get(MetricsKey.INPUT_TOKENS)
            if isinstance(v, int):
                input_tokens = v
            v = total.get(MetricsKey.OUTPUT_TOKENS)
            if isinstance(v, int):
                output_tokens = v
            v = total.get(MetricsKey.TOTAL_TOKENS)
            if isinstance(v, int):
                total_tokens = v
            v = total.get(MetricsKey.CACHED_INPUT_TOKENS)
            if isinstance(v, int):
                cached_input_tokens = v
            v = total.get(MetricsKey.REASONING_OUTPUT_TOKENS)
            if isinstance(v, int):
                reasoning_output_tokens = v

            vllm_graphs = _backend_graphs(metrics)
            if vllm_graphs:
                pb = sum(g.get(MetricsKey.PINNED_BLOCKS, 0) for g in vllm_graphs)
                ph = sum(g.get(MetricsKey.PINNED_HANDLES, 0) for g in vllm_graphs)
                cpl = sum(g.get(MetricsKey.CRITICAL_PATH_LENGTH, 0) for g in vllm_graphs)
                pinned_blocks = pb
                pinned_handles = ph
                critical_path_length = cpl

        return cls(
            graph_duration_ms=graph_duration_ms,
            llm_call_count=llm_call_count,
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            total_tokens=total_tokens,
            cached_input_tokens=cached_input_tokens,
            reasoning_output_tokens=reasoning_output_tokens,
            pinned_blocks=pinned_blocks,
            pinned_handles=pinned_handles,
            critical_path_length=critical_path_length,
            final_output=final_output,
        )


@dataclass
class RunRecord:
    timestamp_utc: str
    graph: str
    mode: str
    backend_label: str
    opt_level: int
    run_index: int
    trial_id: int
    run_order: int
    success: bool
    wall_ms: float
    graph_duration_ms: int | None
    llm_call_count: int | None
    input_tokens: int | None
    output_tokens: int | None
    total_tokens: int | None
    cached_input_tokens: int | None
    reasoning_output_tokens: int | None
    pinned_blocks: int | None
    pinned_handles: int | None
    critical_path_length: int | None
    session_dir: str
    output_excerpt: str
    stderr_excerpt: str

    def as_row(self) -> dict[str, Any]:
        return {
            "timestamp_utc": self.timestamp_utc,
            "graph": self.graph,
            "mode": self.mode,
            "backend_label": self.backend_label,
            "opt_level": self.opt_level,
            "run_index": self.run_index,
            "trial_id": self.trial_id,
            "run_order": self.run_order,
            "success": "true" if self.success else "false",
            "wall_ms": f"{self.wall_ms:.3f}",
            "graph_duration_ms": "" if self.graph_duration_ms is None else self.graph_duration_ms,
            "llm_call_count": "" if self.llm_call_count is None else self.llm_call_count,
            "input_tokens": "" if self.input_tokens is None else self.input_tokens,
            "output_tokens": "" if self.output_tokens is None else self.output_tokens,
            "total_tokens": "" if self.total_tokens is None else self.total_tokens,
            "cached_input_tokens": ""
            if self.cached_input_tokens is None
            else self.cached_input_tokens,
            "reasoning_output_tokens": ""
            if self.reasoning_output_tokens is None
            else self.reasoning_output_tokens,
            "pinned_blocks": "" if self.pinned_blocks is None else self.pinned_blocks,
            "pinned_handles": "" if self.pinned_handles is None else self.pinned_handles,
            "critical_path_length": "" if self.critical_path_length is None else self.critical_path_length,
            "session_dir": self.session_dir,
            "output_excerpt": self.output_excerpt,
            "stderr_excerpt": self.stderr_excerpt,
        }


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graph", type=Path, default=DEFAULT_GRAPH, help="Graph source to run")
    parser.add_argument(
        "--opt-level",
        dest="opt_levels",
        type=int,
        action="append",
        help="Optimization level to benchmark (repeatable, default: 0 and 2)",
    )
    parser.add_argument(
        "--iterations",
        type=int,
        default=3,
        help="Number of runs per optimization level",
    )
    parser.add_argument(
        "--compile-only",
        action="store_true",
        help="Run `dekk apxm compile` instead of `dekk apxm execute`",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help="CSV output path",
    )
    parser.add_argument(
        "--session-base",
        type=Path,
        default=DEFAULT_SESSION_BASE,
        help="Base directory for emitted sessions in execute mode",
    )
    parser.add_argument(
        "--backend-label",
        default="configured",
        help="Reporting label for the configured backend",
    )
    parser.add_argument(
        "--apxm-config",
        type=Path,
        default=None,
        help="APXM config path exported to Dekk and Python graph emission",
    )
    parser.add_argument(
        "--trace",
        default=None,
        help="Optional APXM trace level for execute mode",
    )
    parser.add_argument(
        "--append",
        action="store_true",
        help="Append to an existing CSV instead of overwriting it",
    )
    parser.add_argument(
        "--interleave-opt-levels",
        action="store_true",
        help="Run trial 1 for every opt level before trial 2 to reduce warm-cache bias.",
    )
    return parser.parse_args()


def _collapse_text(text: str, limit: int = 240) -> str:
    collapsed = " ".join(text.split())
    if len(collapsed) <= limit:
        return collapsed
    return collapsed[: limit - 3] + "..."


def _require_dekk() -> None:
    if shutil.which(DEKK) is None:
        raise SystemExit("error: `dekk` is not on PATH")


def _resolve_session_root(session_base: Path) -> Path:
    direct = session_base / FILE_RESULTS
    if direct.exists():
        return session_base

    children = [child for child in session_base.iterdir() if child.is_dir()]
    if len(children) == 1 and (children[0] / FILE_RESULTS).exists():
        return children[0]

    raise FileNotFoundError(
        f"no {FILE_RESULTS} under {session_base} or its single child directory"
    )


def _read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def _command_prefix(apxm_config: Path | None) -> list[str]:
    return [DEKK, APXM]


def _command_env(apxm_config: Path | None) -> dict[str, str] | None:
    if apxm_config is None:
        return None
    env = os.environ.copy()
    env[ENV_APXM_CONFIG] = str(apxm_config)
    return env


def _run_compile(
    graph: Path,
    opt_level: int,
    apxm_config: Path | None,
) -> tuple[subprocess.CompletedProcess[str], float]:
    with tempfile.TemporaryDirectory(prefix="apxm-bench-compile-") as tmp_dir:
        artifact_path = Path(tmp_dir) / f"{graph.stem}-O{opt_level}.apxmobj"
        cmd = [
            *_command_prefix(apxm_config),
            CMD_COMPILE,
            str(graph),
            FLAG_OUTPUT,
            str(artifact_path),
            FLAG_OPT_LEVEL,
            str(opt_level),
        ]
        start = time.perf_counter()
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            env=_command_env(apxm_config),
        )
        wall_ms = (time.perf_counter() - start) * 1000.0
    return result, wall_ms


def _run_execute(
    graph: Path,
    opt_level: int,
    session_base: Path,
    trace: str | None,
    apxm_config: Path | None,
) -> tuple[subprocess.CompletedProcess[str], float]:
    session_base.mkdir(parents=True, exist_ok=True)
    cmd = [
        *_command_prefix(apxm_config),
        CMD_EXECUTE,
        FLAG_OPT_LEVEL,
        str(opt_level),
    ]
    if trace:
        cmd.extend([FLAG_TRACE, trace])
    cmd.extend([FLAG_EMIT_SESSION, str(session_base), str(graph)])

    start = time.perf_counter()
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        env=_command_env(apxm_config),
    )
    wall_ms = (time.perf_counter() - start) * 1000.0
    return result, wall_ms


def _run_once(
    graph: Path,
    opt_level: int,
    run_index: int,
    trial_id: int,
    run_order: int,
    compile_only: bool,
    backend_label: str,
    trace: str | None,
    session_parent: Path,
    apxm_config: Path | None,
) -> RunRecord:
    timestamp = datetime.now(timezone.utc).isoformat()
    mode = "compile" if compile_only else "execute"

    if compile_only:
        result, wall_ms = _run_compile(graph, opt_level, apxm_config)
        return RunRecord(
            timestamp_utc=timestamp,
            graph=str(graph),
            mode=mode,
            backend_label=backend_label,
            opt_level=opt_level,
            run_index=run_index,
            trial_id=trial_id,
            run_order=run_order,
            success=result.returncode == 0,
            wall_ms=wall_ms,
            graph_duration_ms=None,
            llm_call_count=None,
            input_tokens=None,
            output_tokens=None,
            total_tokens=None,
            cached_input_tokens=None,
            reasoning_output_tokens=None,
            pinned_blocks=None,
            pinned_handles=None,
            critical_path_length=None,
            session_dir="",
            output_excerpt="",
            stderr_excerpt=_collapse_text(result.stderr or result.stdout),
        )

    session_base = session_parent / f"opt-{opt_level}" / f"run-{run_index}"
    result, wall_ms = _run_execute(graph, opt_level, session_base, trace, apxm_config)
    session_root: Path | None = None
    summary: SessionSummary | None = None
    if result.returncode == 0:
        session_root = _resolve_session_root(session_base)
        summary = SessionSummary.from_session(session_root)
    return RunRecord(
        timestamp_utc=timestamp,
        graph=str(graph),
        mode=mode,
        backend_label=backend_label,
        opt_level=opt_level,
        run_index=run_index,
        trial_id=trial_id,
        run_order=run_order,
        success=result.returncode == 0,
        wall_ms=wall_ms,
        graph_duration_ms=summary.graph_duration_ms if summary else None,
        llm_call_count=summary.llm_call_count if summary else None,
        input_tokens=summary.input_tokens if summary else None,
        output_tokens=summary.output_tokens if summary else None,
        total_tokens=summary.total_tokens if summary else None,
        cached_input_tokens=summary.cached_input_tokens if summary else None,
        reasoning_output_tokens=summary.reasoning_output_tokens if summary else None,
        pinned_blocks=summary.pinned_blocks if summary else None,
        pinned_handles=summary.pinned_handles if summary else None,
        critical_path_length=summary.critical_path_length if summary else None,
        session_dir=str(session_root) if session_root is not None else "",
        output_excerpt=_collapse_text(summary.final_output if summary else ""),
        stderr_excerpt=_collapse_text(result.stderr or result.stdout),
    )


def _write_csv(records: list[RunRecord], output: Path, append: bool) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    mode = "a" if append and output.exists() else "w"
    write_header = mode == "w"
    with output.open(mode, newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=CSV_FIELDS)
        if write_header:
            writer.writeheader()
        for record in records:
            writer.writerow(record.as_row())


def _summarize(records: list[RunRecord]) -> None:
    by_opt: dict[int, list[RunRecord]] = {}
    for record in records:
        by_opt.setdefault(record.opt_level, []).append(record)

    print("# benchmark_e2e summary")
    for opt_level in sorted(by_opt):
        rows = by_opt[opt_level]
        successes = [row for row in rows if row.success]
        wall_values = [row.wall_ms for row in successes]
        if wall_values:
            mean_wall = statistics.fmean(wall_values)
            median_wall = statistics.median(wall_values)
            graph_values = [
                row.graph_duration_ms for row in successes if row.graph_duration_ms is not None
            ]
            token_values = [
                row.total_tokens for row in successes if row.total_tokens is not None
            ]
            call_values = [
                row.llm_call_count for row in successes if row.llm_call_count is not None
            ]
            metric_parts = [
                f"mean wall {mean_wall:.1f} ms",
                f"median wall {median_wall:.1f} ms",
            ]
            if graph_values:
                metric_parts.append(f"mean graph {statistics.fmean(graph_values):.1f} ms")
            if token_values:
                metric_parts.append(f"mean tokens {statistics.fmean(token_values):.1f}")
            if call_values:
                metric_parts.append(f"mean LLM calls {statistics.fmean(call_values):.1f}")
            print(
                f"- O{opt_level}: {len(successes)}/{len(rows)} succeeded, "
                + ", ".join(metric_parts)
            )
        else:
            print(f"- O{opt_level}: 0/{len(rows)} succeeded")


def _display_number(value: int | float | None, suffix: str = "") -> str:
    if value is None:
        return PENDING_VALUE
    if isinstance(value, int):
        return f"{value}{suffix}"
    return f"{value:.1f}{suffix}"


def _print_run_summary(record: RunRecord) -> None:
    status = STATUS_SUCCEEDED if record.success else STATUS_FAILED
    print(
        f"- {RUN_SUMMARY_PREFIX}: O{record.opt_level} trial {record.trial_id} "
        f"run {record.run_index} order {record.run_order} {status}; "
        f"wall={record.wall_ms:.1f} ms; "
        f"graph={_display_number(record.graph_duration_ms, ' ms')}; "
        f"llm_calls={_display_number(record.llm_call_count)}; "
        f"tokens={_display_number(record.total_tokens)}; "
        f"session={record.session_dir or PENDING_VALUE}",
        flush=True,
    )


def main() -> int:
    args = _parse_args()
    _require_dekk()

    graph = args.graph.resolve()
    if not graph.exists():
        raise SystemExit(f"error: graph not found: {graph}")
    if args.iterations < 1:
        raise SystemExit("error: --iterations must be >= 1")

    opt_levels = args.opt_levels or [0, 2]
    records: list[RunRecord] = []
    run_order = 0

    if args.interleave_opt_levels:
        run_plan = [
            (opt_level, trial_id, trial_id)
            for trial_id in range(1, args.iterations + 1)
            for opt_level in opt_levels
        ]
    else:
        run_plan = [
            (opt_level, run_index, run_index)
            for opt_level in opt_levels
            for run_index in range(1, args.iterations + 1)
        ]

    for opt_level, run_index, trial_id in run_plan:
        run_order += 1
        record = _run_once(
            graph=graph,
            opt_level=opt_level,
            run_index=run_index,
            trial_id=trial_id,
            run_order=run_order,
            compile_only=args.compile_only,
            backend_label=args.backend_label,
            trace=args.trace,
            session_parent=args.session_base.resolve(),
            apxm_config=args.apxm_config.resolve() if args.apxm_config else None,
        )
        records.append(record)
        _print_run_summary(record)

    _write_csv(records, args.output.resolve(), args.append)
    _summarize(records)
    print(f"CSV written to {args.output.resolve()}")

    return 0 if all(record.success for record in records) else 1


if __name__ == "__main__":
    sys.exit(main())
