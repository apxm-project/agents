#!/usr/bin/env python3
"""Run one of the three demo cases through the shared benchmark harness.

Usage:
    python3 examples/python/demos/gemma4/scripts/run_case.py \\
        --case context-pruning \\
        --run-dir <repo>/.apxm/evaluation/gemma4/runs/<UTC>-three-cases \\
        --apxm-config <repo>/.apxm/evaluation/gemma4/runs/<UTC>-three-cases/generated/config.toml

This wrapper hides the path to the shared `benchmark_e2e.py` harness so
operators never need to know about anything outside `demos/gemma4/`. Each
case has a fixed iteration count, target, and runtime arg.

vLLM cache isolation
--------------------
The shared harness (`benchmarks/benchmark_e2e.py`) automatically sets
`APXM_VLLM_CACHE_SALT=execution` on every `dekk apxm compile/execute` child
invocation. This salts the vLLM prefix cache by execution_id so back-to-back
O0/O2 sweeps stay isolated from each other's KV state. Production runs
(`dekk apxm execute` invoked directly, not through this harness) reuse the
prefix cache across executions as designed.
"""

from __future__ import annotations

import argparse
import csv
import statistics
import subprocess
import sys
from pathlib import Path


CASES = {
    "review-synthesis": {
        "workflow": "01_review_synthesis_skill.py",
        "iterations": 1,
        "target": "balanced",
        "subdir": "review-synthesis",
        "runtime_arg": (
            "Evaluate APXM's value for running a workflow where Claude and Codex "
            "inspect the same engineering problem in parallel, then Gemma 4 on "
            "the registered vLLM backend synthesizes the validation checklist."
        ),
    },
    "context-pruning": {
        "workflow": "02_checkout_context_pruning.py",
        "iterations": 3,
        "target": "tokens",
        "subdir": "context-pruning",
        "runtime_arg": None,
    },
    "backend-hints": {
        "workflow": "03_vllm_backend_hints.py",
        "iterations": 3,
        "target": "latency",
        "subdir": "backend-hints/priority-contention",
        "runtime_arg": None,
    },
}


def _repo_root(start: Path) -> Path:
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / "Cargo.toml").is_file() and (candidate / "crates").is_dir():
            return candidate
    raise SystemExit("error: could not locate APXM repo root")


def _parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--case", required=True, choices=sorted(CASES), help="Which case to run.")
    parser.add_argument("--run-dir", required=True, type=Path, help="Run directory created by run_demo.py.")
    parser.add_argument("--apxm-config", required=True, type=Path, help="Path to generated/config.toml.")
    parser.add_argument("--backend-label", default="gemma4-demo-vllm", help="Backend label stamped into rows.")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = _parse_args(argv)
    spec = CASES[args.case]
    repo_root = _repo_root(Path(__file__))
    demo_root = Path(__file__).resolve().parents[1]
    harness = repo_root / "examples" / "python" / "benchmarks" / "benchmark_e2e.py"
    workflow = demo_root / "workflows" / spec["workflow"]
    case_dir = args.run_dir / spec["subdir"]

    cmd = [
        sys.executable, str(harness),
        "--graph", str(workflow),
        "--iterations", str(spec["iterations"]),
        "--precompile-artifacts",
        "--emit-compiler-diagnostics",
        "--artifact-dir", str(case_dir / "artifacts"),
        "--diagnostics-dir", str(case_dir / "compiler-diagnostics"),
        "--output", str(case_dir / "runtime-o0-o2.csv"),
        "--session-base", str(case_dir / "sessions"),
        "--backend-label", args.backend_label,
        "--target", spec["target"],
        "--apxm-config", str(args.apxm_config),
        "--interleave-opt-levels",
    ]
    if spec["runtime_arg"]:
        cmd.extend(["--runtime-arg", spec["runtime_arg"]])

    case_dir.mkdir(parents=True, exist_ok=True)
    print("$", " ".join(cmd))
    rc = subprocess.call(cmd)

    # Backend-hints is the priority-contention case: the workflow's claim is
    # that compiler-stamped priority hints reduce critical-chain finish time
    # under queue contention, NOT whole-graph wall time. Surface that delta
    # explicitly so the headline does not get drowned out by `wall_ms`, which
    # stays dominated by the four background audit prompts.
    if args.case == "backend-hints":
        csv_path = case_dir / "runtime-o0-o2.csv"
        if csv_path.is_file():
            _print_priority_contention_headline(csv_path)
    return rc


def _print_priority_contention_headline(csv_path: Path) -> None:
    by_opt: dict[str, dict[str, list[float]]] = {}
    try:
        with csv_path.open(newline="") as handle:
            for row in csv.DictReader(handle):
                if row.get("success") != "true":
                    continue
                opt = row.get("opt_level", "")
                bucket = by_opt.setdefault(opt, {"wall": [], "milestone": [], "finish": []})
                wall = row.get("wall_ms", "")
                if wall:
                    try:
                        bucket["wall"].append(float(wall))
                    except ValueError:
                        pass
                ms = row.get("critical_milestone_last_ms", "")
                if ms:
                    try:
                        bucket["milestone"].append(float(ms))
                    except ValueError:
                        pass
                fin = row.get("observed_critical_path_finish_ms", "")
                if fin:
                    try:
                        bucket["finish"].append(float(fin))
                    except ValueError:
                        pass
    except OSError:
        return

    if not by_opt:
        return

    print()
    print("# backend-hints/priority-contention — claim-metric headline")
    print(
        "  (workflow optimizes critical-chain finish time, NOT whole-graph wall;"
        " wall stays dominated by background audit prompts)"
    )

    def _mean(values: list[float]) -> float | None:
        return statistics.fmean(values) if values else None

    rows: list[tuple[str, float | None, float | None, float | None]] = []
    for opt in sorted(by_opt):
        bucket = by_opt[opt]
        rows.append((
            opt,
            _mean(bucket["milestone"]),
            _mean(bucket["finish"]),
            _mean(bucket["wall"]),
        ))

    for opt, milestone, finish, wall in rows:
        parts: list[str] = []
        if milestone is not None:
            parts.append(f"critical milestone last {milestone:.1f} ms")
        if finish is not None:
            parts.append(f"observed critical finish {finish:.1f} ms")
        if wall is not None:
            parts.append(f"wall {wall:.1f} ms (background-dominated)")
        print(f"- O{opt}: " + "; ".join(parts) if parts else f"- O{opt}: <no data>")

    milestone_by_opt = {opt: ms for opt, ms, _, _ in rows if ms is not None}
    if len(milestone_by_opt) >= 2:
        baseline_opt = min(milestone_by_opt)
        baseline = milestone_by_opt[baseline_opt]
        for opt, value in milestone_by_opt.items():
            if opt == baseline_opt:
                continue
            delta = value - baseline
            pct = (delta / baseline * 100.0) if baseline else 0.0
            print(
                f"- O{opt} vs O{baseline_opt} critical-chain delta: "
                f"{delta:+.1f} ms ({pct:+.1f}%)"
            )


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
