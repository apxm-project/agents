#!/usr/bin/env python3
"""Benchmark suite for vLLM optimizations.

Measures the impact of:
- PromptCanonicalization (shared prefix reuse)
- Pipelining (sequential LLM operations)
- Priority scheduling (critical path vs background)
- Per-node model routing (cost/quality tradeoffs)

Usage:
    python3 scripts/benchmark.py --all
    python3 scripts/benchmark.py --graph shared_prefix_fanout --compare O0,O2
    python3 scripts/benchmark.py --graph chained_llm --target latency
    python3 scripts/benchmark.py --list
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Dict, List, Optional, Tuple
from datetime import datetime


# Benchmark configuration
BENCHMARKS = {
    "shared_prefix_fanout": {
        "file": "examples/python/_benchmarks/shared_prefix_fanout.py",
        "description": "Tests prefix reuse optimization (PromptCanonicalization)",
        "metrics": ["duration_ms", "cache_hits"],
    },
    "chained_llm": {
        "file": "examples/python/_benchmarks/chained_llm.py",
        "description": "Tests LLM pipelining optimization",
        "metrics": ["duration_ms", "avg_parallelism"],
    },
    "mixed_priority": {
        "file": "examples/python/_benchmarks/mixed_priority.py",
        "description": "Tests priority-based scheduling",
        "metrics": ["duration_ms", "critical_path_latency"],
    },
    "multi_model": {
        "file": "examples/python/_benchmarks/multi_model.py",
        "description": "Tests per-node model/backend routing",
        "metrics": ["duration_ms", "cost_estimate"],
    },
    "fusion_stress": {
        "file": "examples/python/_benchmarks/fusion_stress.py",
        "description": "Tests FuseAskOps pass (10 sequential ask→think pairs)",
        "metrics": ["duration_ms", "llm_call_count"],
    },
    "cse_stress": {
        "file": "examples/python/_benchmarks/cse_stress.py",
        "description": "Tests Common Subexpression Elimination (3 identical prompts)",
        "metrics": ["duration_ms", "unique_llm_calls", "cache_hit_rate"],
    },
    "dead_context_stress": {
        "file": "examples/python/_benchmarks/dead_context_stress.py",
        "description": "Tests DeadContextElimination (5 contexts, only one used)",
        "metrics": ["duration_ms", "input_tokens", "token_savings_pct"],
    },
    "prefix_fanout_large": {
        "file": "examples/python/_benchmarks/prefix_fanout_large.py",
        "description": "Tests PromptCanonicalization (8-way fanout, 4k shared prefix)",
        "metrics": ["duration_ms", "prefill_tokens", "cache_hit_rate"],
    },
    "priority_scheduling": {
        "file": "examples/python/_benchmarks/priority_scheduling.py",
        "description": "Tests priority scheduling (critical path vs background)",
        "metrics": ["duration_ms", "critical_path_latency"],
    },
    "memo_cache_stress": {
        "file": "examples/python/_benchmarks/memo_cache_stress.py",
        "description": "Tests MemoCache effectiveness (repeated prompts)",
        "metrics": ["duration_ms", "llm_calls", "cache_hit_rate"],
    },
}

PROJECT_ROOT = Path(__file__).parent.parent


def find_latest_session(pattern: str) -> Optional[Path]:
    """Find the most recent session directory matching pattern."""
    sessions_dir = Path.home() / ".apxm" / "sessions"
    if not sessions_dir.exists():
        return None

    matching = sorted(
        [d for d in sessions_dir.iterdir() if d.is_dir() and pattern in d.name],
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )
    return matching[0] if matching else None


def generate_graph(benchmark_file: Path) -> Optional[Path]:
    """Generate .air graph from Python file."""
    print(f"  Generating graph from {benchmark_file.name}...", end=" ")

    try:
        # Add Python frontend to path
        python_frontend = PROJECT_ROOT / "crates" / "apxm-frontend" / "python"
        env = dict(os.environ)
        env["PYTHONPATH"] = f"{python_frontend}:{env.get('PYTHONPATH', '')}"

        # Run the Python file to generate AIR JSON
        result = subprocess.run(
            ["python3", str(benchmark_file)],
            capture_output=True,
            text=True,
            timeout=30,
            env=env,
        )

        if result.returncode != 0:
            print(f"FAILED")
            print(f"  Error: {result.stderr}")
            return None

        # Parse the AIR output
        air_json = result.stdout.strip()
        graph_data = json.loads(air_json)

        # Write to .air file
        output_file = benchmark_file.with_suffix(".air")
        with open(output_file, "w") as f:
            json.dump(graph_data, f, indent=2)

        print(f"OK → {output_file.name}")
        return output_file

    except subprocess.TimeoutExpired:
        print("TIMEOUT")
        return None
    except json.JSONDecodeError as e:
        print(f"FAILED (invalid JSON: {e})")
        return None
    except Exception as e:
        print(f"FAILED ({e})")
        return None


def compile_graph(graph_file: Path, opt_level: int = 2) -> Optional[Path]:
    """Compile .air graph to .apxmobj."""
    artifact_file = graph_file.with_suffix(".apxmobj")
    print(f"  Compiling -O{opt_level}...", end=" ")

    try:
        cmd = [
            "dekk",
            "apxm",
            "compile",
            str(graph_file),
            f"-O{opt_level}",
            "-o",
            str(artifact_file),
        ]

        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=120,
            cwd=PROJECT_ROOT,
        )

        if result.returncode != 0:
            print(f"FAILED")
            print(f"  Error: {result.stderr}")
            return None

        print(f"OK")
        return artifact_file

    except subprocess.TimeoutExpired:
        print("TIMEOUT")
        return None
    except Exception as e:
        print(f"FAILED ({e})")
        return None


def execute_artifact(
    artifact_file: Path,
    emit_session: bool = True,
    target: Optional[str] = None,
    use_mock: bool = False,
    mock_latency: int = 500,
) -> Optional[Dict]:
    """Execute compiled artifact and return metrics."""
    print(f"  Executing artifact...", end=" ", flush=True)

    try:
        cmd = ["dekk", "apxm", "run", str(artifact_file)]

        if emit_session:
            cmd.append("--emit-session")

        if target:
            cmd.extend(["--target", target])

        # Set up environment for mock backend
        env = dict(os.environ)
        if use_mock:
            env["APXM_MOCK_BACKEND"] = "1"
            env["APXM_MOCK_LATENCY_MS"] = str(mock_latency)

        start_time = time.time()
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=300,  # 5 minutes max
            cwd=PROJECT_ROOT,
            env=env,
        )
        wall_time_ms = (time.time() - start_time) * 1000

        if result.returncode != 0:
            print(f"FAILED")
            print(f"  Error: {result.stderr}")
            return None

        print(f"OK ({wall_time_ms:.0f}ms)")

        # Find the session directory
        graph_name = artifact_file.stem.replace(".apxmobj", "")
        session_dir = find_latest_session(graph_name)

        if not session_dir or not emit_session:
            return {"wall_time_ms": wall_time_ms}

        # Read metrics from session
        metrics_file = session_dir / "metrics.json"
        manifest_file = session_dir / "manifest.json"

        metrics = {"wall_time_ms": wall_time_ms}

        if metrics_file.exists():
            with open(metrics_file) as f:
                metrics.update(json.load(f))

        if manifest_file.exists():
            with open(manifest_file) as f:
                manifest = json.load(f)
                metrics["manifest"] = manifest

        return metrics

    except subprocess.TimeoutExpired:
        print("TIMEOUT")
        return None
    except Exception as e:
        print(f"FAILED ({e})")
        return None


def run_benchmark(
    name: str,
    config: Dict,
    opt_levels: List[int],
    targets: Optional[List[str]] = None,
    use_mock: bool = False,
    mock_latency: int = 500,
) -> Dict[str, Dict]:
    """Run a single benchmark with multiple configurations."""
    print(f"\n{'='*60}")
    print(f"Benchmark: {name}")
    print(f"Description: {config['description']}")
    if use_mock:
        print(f"Mode: MOCK (latency={mock_latency}ms)")
    print(f"{'='*60}")

    benchmark_file = PROJECT_ROOT / config["file"]
    if not benchmark_file.exists():
        print(f"ERROR: Benchmark file not found: {benchmark_file}")
        return {}

    # Step 1: Generate graph
    graph_file = generate_graph(benchmark_file)
    if not graph_file:
        return {}

    results = {}

    # Step 2: Run with different optimization levels
    for opt in opt_levels:
        artifact = compile_graph(graph_file, opt_level=opt)
        if not artifact:
            continue

        # Run with different targets if specified
        if targets:
            for target in targets:
                key = f"O{opt}-{target}"
                print(f"  Configuration: {key}")
                metrics = execute_artifact(
                    artifact,
                    target=target,
                    use_mock=use_mock,
                    mock_latency=mock_latency,
                )
                if metrics:
                    results[key] = metrics
        else:
            key = f"O{opt}"
            print(f"  Configuration: {key}")
            metrics = execute_artifact(
                artifact,
                use_mock=use_mock,
                mock_latency=mock_latency,
            )
            if metrics:
                results[key] = metrics

    return results


def format_results_table(
    all_results: Dict[str, Dict[str, Dict]]
) -> str:
    """Format benchmark results as markdown table."""
    lines = []
    lines.append("\n" + "=" * 80)
    lines.append("BENCHMARK RESULTS")
    lines.append("=" * 80)
    lines.append("")

    for bench_name, configs in all_results.items():
        if not configs:
            continue

        lines.append(f"\n## {bench_name}")
        lines.append(f"Description: {BENCHMARKS[bench_name]['description']}")
        lines.append("")

        # Build table header
        config_keys = list(configs.keys())
        lines.append(f"| Metric | {' | '.join(config_keys)} |")
        lines.append(f"|--------|{'----|' * len(config_keys)}")

        # Extract metrics
        all_metrics = {}
        for config, data in configs.items():
            exec_data = data.get("execution", {})
            manifest = data.get("manifest", {})

            all_metrics[config] = {
                "duration_ms": exec_data.get("duration_ms", manifest.get("duration_ms", 0)),
                "nodes_executed": exec_data.get("nodes_executed", manifest.get("node_count", 0)),
                "wall_time_ms": data.get("wall_time_ms", 0),
            }

            sched = data.get("scheduler", {})
            if sched:
                all_metrics[config]["avg_parallelism"] = sched.get("avg_parallelism", 0)
                all_metrics[config]["max_parallelism"] = sched.get("max_parallelism", 0)

        # Build rows
        metric_names = set()
        for config_data in all_metrics.values():
            metric_names.update(config_data.keys())

        for metric in sorted(metric_names):
            values = []
            for config in config_keys:
                val = all_metrics.get(config, {}).get(metric, "N/A")
                if isinstance(val, float):
                    values.append(f"{val:.2f}")
                elif isinstance(val, int):
                    values.append(f"{val}")
                else:
                    values.append(str(val))

            lines.append(f"| {metric} | {' | '.join(values)} |")

        # Calculate speedup if we have O0 vs O2
        if "O0" in configs and "O2" in configs:
            o0_time = all_metrics["O0"]["duration_ms"]
            o2_time = all_metrics["O2"]["duration_ms"]
            if o0_time > 0 and o2_time > 0:
                speedup = o0_time / o2_time
                lines.append(f"| **Speedup (O2/O0)** | **{speedup:.2f}x** | |")

        lines.append("")

    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(
        description="Benchmark suite for vLLM optimizations"
    )
    parser.add_argument(
        "--all", action="store_true", help="Run all benchmarks"
    )
    parser.add_argument(
        "--graph",
        choices=list(BENCHMARKS.keys()),
        help="Run specific benchmark",
    )
    parser.add_argument(
        "--compare",
        help="Comma-separated optimization levels to compare (e.g., O0,O2)",
    )
    parser.add_argument(
        "--target",
        choices=["latency", "cost", "quality"],
        help="Optimization target",
    )
    parser.add_argument(
        "--list", action="store_true", help="List available benchmarks"
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Write results to file (default: stdout)",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Use mock LLM backend for reproducible benchmarks",
    )
    parser.add_argument(
        "--mock-latency",
        type=int,
        default=500,
        help="Mock backend latency in milliseconds (default: 500)",
    )

    args = parser.parse_args()

    # List benchmarks
    if args.list:
        print("\nAvailable benchmarks:\n")
        for name, config in BENCHMARKS.items():
            print(f"  {name}")
            print(f"    {config['description']}")
        print()
        return 0

    # Determine which benchmarks to run
    if args.all:
        benchmarks_to_run = list(BENCHMARKS.keys())
    elif args.graph:
        benchmarks_to_run = [args.graph]
    else:
        parser.print_help()
        return 1

    # Determine optimization levels
    if args.compare:
        opt_levels = []
        for level_str in args.compare.split(","):
            if level_str.startswith("O"):
                opt_levels.append(int(level_str[1:]))
            else:
                opt_levels.append(int(level_str))
    else:
        opt_levels = [0, 2]  # Default: compare O0 vs O2

    # Determine targets
    targets = [args.target] if args.target else None

    # Run benchmarks
    all_results = {}
    for name in benchmarks_to_run:
        config = BENCHMARKS[name]
        results = run_benchmark(
            name,
            config,
            opt_levels,
            targets,
            use_mock=args.mock,
            mock_latency=args.mock_latency,
        )
        all_results[name] = results

    # Format and output results
    report = format_results_table(all_results)

    if args.output:
        with open(args.output, "w") as f:
            f.write(report)
        print(f"\nResults written to: {args.output}")
    else:
        print(report)

    # Also save raw JSON
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    json_file = PROJECT_ROOT / f"benchmark_results_{timestamp}.json"
    with open(json_file, "w") as f:
        json.dump(all_results, f, indent=2)
    print(f"\nRaw data saved to: {json_file}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
