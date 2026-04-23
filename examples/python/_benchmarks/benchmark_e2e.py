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
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
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

FILE_MANIFEST = "manifest.json"
FILE_RESULTS = "results.json"
FILE_METRICS = "metrics.json"

KEY_DURATION_MS = "duration_ms"
KEY_FINAL_OUTPUT = "final_output"
KEY_TOKEN_ACCOUNTING = "token_accounting"
KEY_TOTAL = "total"
KEY_INPUT_TOKENS = "input_tokens"
KEY_OUTPUT_TOKENS = "output_tokens"
KEY_TOTAL_TOKENS = "total_tokens"
KEY_CALL_COUNT = "call_count"

CSV_FIELDS = [
    "timestamp_utc",
    "graph",
    "mode",
    "backend_label",
    "opt_level",
    "run_index",
    "success",
    "wall_ms",
    "graph_duration_ms",
    "llm_call_count",
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "session_dir",
    "output_excerpt",
    "stderr_excerpt",
]


@dataclass
class RunRecord:
    timestamp_utc: str
    graph: str
    mode: str
    backend_label: str
    opt_level: int
    run_index: int
    success: bool
    wall_ms: float
    graph_duration_ms: int | None
    llm_call_count: int | None
    input_tokens: int | None
    output_tokens: int | None
    total_tokens: int | None
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
            "success": "true" if self.success else "false",
            "wall_ms": f"{self.wall_ms:.3f}",
            "graph_duration_ms": "" if self.graph_duration_ms is None else self.graph_duration_ms,
            "llm_call_count": "" if self.llm_call_count is None else self.llm_call_count,
            "input_tokens": "" if self.input_tokens is None else self.input_tokens,
            "output_tokens": "" if self.output_tokens is None else self.output_tokens,
            "total_tokens": "" if self.total_tokens is None else self.total_tokens,
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
        "--trace",
        default=None,
        help="Optional APXM trace level for execute mode",
    )
    parser.add_argument(
        "--append",
        action="store_true",
        help="Append to an existing CSV instead of overwriting it",
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


def _read_session_summary(session_root: Path) -> dict[str, Any]:
    summary: dict[str, Any] = {
        "graph_duration_ms": None,
        "llm_call_count": None,
        "input_tokens": None,
        "output_tokens": None,
        "total_tokens": None,
        "final_output": "",
    }

    manifest_path = session_root / FILE_MANIFEST
    results_path = session_root / FILE_RESULTS
    metrics_path = session_root / FILE_METRICS

    if manifest_path.exists():
        manifest = _read_json(manifest_path)
        duration = manifest.get(KEY_DURATION_MS)
        if isinstance(duration, int):
            summary["graph_duration_ms"] = duration

    if results_path.exists():
        results = _read_json(results_path)
        final_output = results.get(KEY_FINAL_OUTPUT)
        if isinstance(final_output, str):
            summary["final_output"] = final_output

    if metrics_path.exists():
        metrics = _read_json(metrics_path)
        total = (
            metrics.get(KEY_TOKEN_ACCOUNTING, {})
            .get(KEY_TOTAL, {})
        )
        for key in (KEY_CALL_COUNT, KEY_INPUT_TOKENS, KEY_OUTPUT_TOKENS, KEY_TOTAL_TOKENS):
            value = total.get(key)
            if isinstance(value, int):
                if key == KEY_CALL_COUNT:
                    summary["llm_call_count"] = value
                elif key == KEY_INPUT_TOKENS:
                    summary["input_tokens"] = value
                elif key == KEY_OUTPUT_TOKENS:
                    summary["output_tokens"] = value
                elif key == KEY_TOTAL_TOKENS:
                    summary["total_tokens"] = value

    return summary


def _run_compile(graph: Path, opt_level: int) -> tuple[subprocess.CompletedProcess[str], float]:
    with tempfile.TemporaryDirectory(prefix="apxm-bench-compile-") as tmp_dir:
        artifact_path = Path(tmp_dir) / f"{graph.stem}-O{opt_level}.apxmobj"
        cmd = [
            DEKK,
            APXM,
            CMD_COMPILE,
            str(graph),
            FLAG_OUTPUT,
            str(artifact_path),
            FLAG_OPT_LEVEL,
            str(opt_level),
        ]
        start = time.perf_counter()
        result = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO_ROOT)
        wall_ms = (time.perf_counter() - start) * 1000.0
    return result, wall_ms


def _run_execute(
    graph: Path,
    opt_level: int,
    session_base: Path,
    trace: str | None,
) -> tuple[subprocess.CompletedProcess[str], float]:
    session_base.mkdir(parents=True, exist_ok=True)
    cmd = [
        DEKK,
        APXM,
        CMD_EXECUTE,
        FLAG_OPT_LEVEL,
        str(opt_level),
    ]
    if trace:
        cmd.extend([FLAG_TRACE, trace])
    cmd.extend([FLAG_EMIT_SESSION, str(session_base), str(graph)])

    start = time.perf_counter()
    result = subprocess.run(cmd, capture_output=True, text=True, cwd=REPO_ROOT)
    wall_ms = (time.perf_counter() - start) * 1000.0
    return result, wall_ms


def _run_once(
    graph: Path,
    opt_level: int,
    run_index: int,
    compile_only: bool,
    backend_label: str,
    trace: str | None,
    session_parent: Path,
) -> RunRecord:
    timestamp = datetime.now(timezone.utc).isoformat()
    mode = "compile" if compile_only else "execute"

    if compile_only:
        result, wall_ms = _run_compile(graph, opt_level)
        return RunRecord(
            timestamp_utc=timestamp,
            graph=str(graph),
            mode=mode,
            backend_label=backend_label,
            opt_level=opt_level,
            run_index=run_index,
            success=result.returncode == 0,
            wall_ms=wall_ms,
            graph_duration_ms=None,
            llm_call_count=None,
            input_tokens=None,
            output_tokens=None,
            total_tokens=None,
            session_dir="",
            output_excerpt="",
            stderr_excerpt=_collapse_text(result.stderr or result.stdout),
        )

    session_base = session_parent / f"opt-{opt_level}" / f"run-{run_index}"
    result, wall_ms = _run_execute(graph, opt_level, session_base, trace)
    session_root: Path | None = None
    session_summary: dict[str, Any] = {}
    if result.returncode == 0:
        session_root = _resolve_session_root(session_base)
        session_summary = _read_session_summary(session_root)
    return RunRecord(
        timestamp_utc=timestamp,
        graph=str(graph),
        mode=mode,
        backend_label=backend_label,
        opt_level=opt_level,
        run_index=run_index,
        success=result.returncode == 0,
        wall_ms=wall_ms,
        graph_duration_ms=session_summary.get("graph_duration_ms"),
        llm_call_count=session_summary.get("llm_call_count"),
        input_tokens=session_summary.get("input_tokens"),
        output_tokens=session_summary.get("output_tokens"),
        total_tokens=session_summary.get("total_tokens"),
        session_dir=str(session_root) if session_root is not None else "",
        output_excerpt=_collapse_text(session_summary.get("final_output", "")),
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
            print(
                f"- O{opt_level}: {len(successes)}/{len(rows)} succeeded, "
                f"mean wall {mean_wall:.1f} ms, median wall {median_wall:.1f} ms"
            )
        else:
            print(f"- O{opt_level}: 0/{len(rows)} succeeded")


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

    for opt_level in opt_levels:
        for run_index in range(1, args.iterations + 1):
            record = _run_once(
                graph=graph,
                opt_level=opt_level,
                run_index=run_index,
                compile_only=args.compile_only,
                backend_label=args.backend_label,
                trace=args.trace,
                session_parent=args.session_base.resolve(),
            )
            records.append(record)

    _write_csv(records, args.output.resolve(), args.append)
    _summarize(records)
    print(f"CSV written to {args.output.resolve()}")

    return 0 if all(record.success for record in records) else 1


if __name__ == "__main__":
    sys.exit(main())
