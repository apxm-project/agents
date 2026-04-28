#!/usr/bin/env python3
"""Run one of the three demo cases through the shared benchmark harness.

Usage:
    python3 examples/python/demos/gemma4/scripts/run_case.py \\
        --case context-pruning \\
        --run-dir <path>/runs/<UTC>-three-cases \\
        --apxm-config <path>/runs/<UTC>-three-cases/generated/config.toml

This wrapper hides the path to the shared `benchmark_e2e.py` harness so
operators never need to know about anything outside `demos/gemma4/`. Each
case has a fixed iteration count, target, and runtime arg.
"""

from __future__ import annotations

import argparse
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
    parser.add_argument("--backend-label", default="strategic-demo-vllm", help="Backend label stamped into rows.")
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
    return subprocess.call(cmd)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
