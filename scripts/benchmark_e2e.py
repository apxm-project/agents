#!/usr/bin/env python3
"""End-to-end benchmark measuring BOTH compiler AND runtime optimizations.

This benchmark measures:
1. COMPILE PHASE: Node/token reduction, artifact size, compilation time
2. EXECUTION PHASE: Wall time, LLM calls, tokens, cache hits, parallelism
3. COMPARISON: O0 vs O2 to quantify optimization impact

The key insight:
- Compiler optimizations reduce WORK (fewer nodes, fewer tokens)
- Inference optimizations reduce COST OF WORK (faster serving, cache reuse)

Usage:
    # Run all benchmarks in mock mode (reproducible)
    python3 scripts/benchmark_e2e.py --all --mock

    # Run specific benchmark with real LLM
    python3 scripts/benchmark_e2e.py --graph shared_prefix_fanout --real

    # Run with custom mock latency
    python3 scripts/benchmark_e2e.py --graph chained_llm --mock --mock-latency 200

    # Save to specific output file
    python3 scripts/benchmark_e2e.py --all --mock -o results/e2e_benchmark.md
"""

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional, Tuple

PROJECT_ROOT = Path(__file__).parent.parent


@dataclass
class CompileMetrics:
    """Metrics from compilation phase."""
    nodes_before: int
    nodes_after: int
    tokens_before: int
    tokens_after: int
    artifact_size: int
    compile_time_ms: float
    optimizations_applied: List[str]


@dataclass
class ExecutionMetrics:
    """Metrics from execution phase."""
    wall_time_ms: float
    execution_time_ms: int
    nodes_executed: int
    nodes_failed: int
    llm_calls: int
    input_tokens: int
    output_tokens: int
    cache_hits: int
    avg_parallelism: float
    max_parallelism: int
    critical_path_ms: Optional[float]


@dataclass
class BenchmarkResult:
    """Complete benchmark result for one configuration."""
    opt_level: int
    compile: CompileMetrics
    execution: ExecutionMetrics


# Benchmark configurations
BENCHMARKS = {
    "shared_prefix_fanout": {
        "file": "examples/python/_benchmarks/shared_prefix_fanout.py",
        "description": "Tests prefix reuse optimization (4 parallel ASK with shared context)",
        "expected_improvement": "O2 should deduplicate shared prefix, reducing tokens sent",
    },
    "chained_llm": {
        "file": "examples/python/_benchmarks/chained_llm.py",
        "description": "Tests sequential ASK chain optimization",
        "expected_improvement": "O2 may fuse operations or optimize data flow",
    },
    "mixed_priority": {
        "file": "examples/python/_benchmarks/mixed_priority.py",
        "description": "Tests priority-based scheduling with parallel branches",
        "expected_improvement": "O2 should optimize scheduling for critical path",
    },
    "multi_model": {
        "file": "examples/python/_benchmarks/multi_model.py",
        "description": "Tests multi-model routing optimization",
        "expected_improvement": "O2 should optimize model selection strategy",
    },
}


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


def generate_graph_from_python(python_file: Path) -> Optional[Path]:
    """Generate .apxm graph from Python file using AIR frontend."""
    print(f"  Generating graph from {python_file.name}...", end=" ", flush=True)

    try:
        # Add Python frontend to path
        python_frontend = PROJECT_ROOT / "crates" / "apxm-frontend" / "python"
        env = dict(os.environ)
        env["PYTHONPATH"] = f"{python_frontend}:{env.get('PYTHONPATH', '')}"

        # Run the Python file to generate AIR JSON
        result = subprocess.run(
            ["python3", str(python_file)],
            capture_output=True,
            text=True,
            timeout=30,
            env=env,
            cwd=PROJECT_ROOT,
        )

        if result.returncode != 0:
            print(f"FAILED")
            print(f"  Error: {result.stderr}")
            return None

        # Parse the AIR output
        air_json = result.stdout.strip()
        graph_data = json.loads(air_json)

        # Write to .apxm file
        output_file = python_file.with_suffix(".apxm")
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


def count_nodes_in_graph(graph_file: Path) -> int:
    """Count nodes in a graph JSON file."""
    try:
        with open(graph_file) as f:
            graph = json.load(f)
        return len(graph.get("nodes", []))
    except Exception as e:
        print(f"  Warning: Could not count nodes in {graph_file}: {e}")
        return 0


def estimate_tokens_in_graph(graph_file: Path) -> int:
    """Estimate token count in graph prompts (rough approximation)."""
    try:
        with open(graph_file) as f:
            graph = json.load(f)

        total_chars = 0
        for node in graph.get("nodes", []):
            attrs = node.get("attributes", {})
            # Count template_str, prompt, and other text fields
            for key, value in attrs.items():
                if isinstance(value, str):
                    total_chars += len(value)

        # Rough approximation: 1 token ~= 4 characters
        return total_chars // 4
    except Exception as e:
        print(f"  Warning: Could not estimate tokens in {graph_file}: {e}")
        return 0


def decompile_artifact(artifact_file: Path) -> Optional[Path]:
    """Decompile artifact back to graph JSON for analysis."""
    try:
        output_file = artifact_file.with_suffix(".decompiled.json")
        cmd = ["dekk", "apxm", "decompile", str(artifact_file)]

        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=30,
            cwd=PROJECT_ROOT,
        )

        if result.returncode != 0:
            return None

        # Parse and save decompiled graph
        graph_data = json.loads(result.stdout)
        with open(output_file, "w") as f:
            json.dump(graph_data, f, indent=2)

        return output_file
    except Exception as e:
        print(f"  Warning: Could not decompile {artifact_file}: {e}")
        return None


def compile_and_measure(
    graph_file: Path, opt_level: int, use_mock: bool = False
) -> Optional[CompileMetrics]:
    """Compile graph and measure compilation metrics."""
    artifact_file = graph_file.with_suffix(f".O{opt_level}.apxmobj")

    print(f"  Compiling -O{opt_level}...", end=" ", flush=True)

    # Count nodes/tokens before optimization
    nodes_before = count_nodes_in_graph(graph_file)
    tokens_before = estimate_tokens_in_graph(graph_file)

    try:
        cmd = [
            "dekk", "apxm", "compile",
            str(graph_file),
            f"-O{opt_level}",
            "-o", str(artifact_file),
        ]

        # Set up environment
        env = dict(os.environ)
        if use_mock:
            # Temporarily hide config so graphs compile without backend references
            config_file = Path.home() / ".apxm" / "config.toml"
            config_backup = Path.home() / ".apxm" / "config.toml.benchmark_backup"
            if config_file.exists():
                config_file.rename(config_backup)
                env["APXM_NO_CONFIG"] = "1"  # Signal to avoid creating default config

        try:
            start_time = time.time()
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=120,
                cwd=PROJECT_ROOT,
                env=env,
            )
            compile_time_ms = (time.time() - start_time) * 1000
        finally:
            # Restore config
            if use_mock and config_backup.exists():
                config_backup.rename(config_file)

        if result.returncode != 0:
            print(f"FAILED")
            print(f"  Error: {result.stderr}")
            return None

        # Get artifact size
        artifact_size = artifact_file.stat().st_size if artifact_file.exists() else 0

        # Decompile to count optimized nodes
        decompiled = decompile_artifact(artifact_file)
        nodes_after = count_nodes_in_graph(decompiled) if decompiled else nodes_before
        tokens_after = estimate_tokens_in_graph(decompiled) if decompiled else tokens_before

        # Extract optimization info from compilation output
        optimizations = []
        if "FuseReasoning" in result.stderr or "Fusion" in result.stderr:
            optimizations.append("FuseReasoning")
        if "DeadCode" in result.stderr:
            optimizations.append("DeadCodeElimination")
        if "CSE" in result.stderr or "CommonSubexpression" in result.stderr:
            optimizations.append("CSE")

        print(f"OK ({compile_time_ms:.0f}ms, {artifact_size} bytes)")

        return CompileMetrics(
            nodes_before=nodes_before,
            nodes_after=nodes_after,
            tokens_before=tokens_before,
            tokens_after=tokens_after,
            artifact_size=artifact_size,
            compile_time_ms=compile_time_ms,
            optimizations_applied=optimizations,
        )

    except subprocess.TimeoutExpired:
        print("TIMEOUT")
        return None
    except Exception as e:
        print(f"FAILED ({e})")
        return None


def execute_and_measure(
    artifact_file: Path,
    use_mock: bool = False,
    mock_latency: int = 500,
) -> Optional[ExecutionMetrics]:
    """Execute artifact and measure runtime metrics."""
    print(f"  Executing...", end=" ", flush=True)

    try:
        cmd = ["dekk", "apxm", "run", str(artifact_file), "--emit-session"]

        # Set up environment
        env = dict(os.environ)
        config_backup = None
        if use_mock:
            env["APXM_MOCK_BACKEND"] = "1"
            env["APXM_MOCK_LATENCY_MS"] = str(mock_latency)
            # Temporarily hide config so mock backend is used
            config_file = Path.home() / ".apxm" / "config.toml"
            config_backup = Path.home() / ".apxm" / "config.toml.benchmark_backup"
            if config_file.exists():
                config_file.rename(config_backup)

        try:
            start_time = time.time()
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=300,
                cwd=PROJECT_ROOT,
                env=env,
            )
            wall_time_ms = (time.time() - start_time) * 1000
        finally:
            # Restore config
            if config_backup and config_backup.exists():
                config_backup.rename(Path.home() / ".apxm" / "config.toml")

        if result.returncode != 0:
            print(f"FAILED")
            print(f"  Error: {result.stderr}")
            return None

        # Find session directory
        graph_name = artifact_file.stem.replace(".apxmobj", "").replace(".O0", "").replace(".O2", "")
        session_dir = find_latest_session(graph_name)

        if not session_dir:
            print(f"OK ({wall_time_ms:.0f}ms) [no session data]")
            return ExecutionMetrics(
                wall_time_ms=wall_time_ms,
                execution_time_ms=int(wall_time_ms),
                nodes_executed=0,
                nodes_failed=0,
                llm_calls=0,
                input_tokens=0,
                output_tokens=0,
                cache_hits=0,
                avg_parallelism=0.0,
                max_parallelism=0,
                critical_path_ms=None,
            )

        # Read session metrics
        manifest_file = session_dir / "manifest.json"
        metrics_file = session_dir / "metrics.json"

        manifest = {}
        if manifest_file.exists():
            with open(manifest_file) as f:
                manifest = json.load(f)

        metrics_data = {}
        if metrics_file.exists():
            with open(metrics_file) as f:
                metrics_data = json.load(f)

        execution_data = metrics_data.get("execution", {})
        scheduler_data = metrics_data.get("scheduler", {})

        print(f"OK ({wall_time_ms:.0f}ms)")

        return ExecutionMetrics(
            wall_time_ms=wall_time_ms,
            execution_time_ms=manifest.get("duration_ms", int(wall_time_ms)),
            nodes_executed=execution_data.get("nodes_executed", manifest.get("node_count", 0)),
            nodes_failed=execution_data.get("nodes_failed", 0),
            llm_calls=metrics_data.get("llm_calls", 0),
            input_tokens=metrics_data.get("input_tokens", 0),
            output_tokens=metrics_data.get("output_tokens", 0),
            cache_hits=metrics_data.get("cache_hits", 0),
            avg_parallelism=scheduler_data.get("avg_parallelism", 0.0),
            max_parallelism=scheduler_data.get("max_parallelism", 0),
            critical_path_ms=scheduler_data.get("critical_path_ms"),
        )

    except subprocess.TimeoutExpired:
        print("TIMEOUT")
        return None
    except Exception as e:
        print(f"FAILED ({e})")
        return None


def run_benchmark(
    name: str,
    graph_file: Path,
    use_mock: bool = False,
    mock_latency: int = 500,
) -> Dict[str, BenchmarkResult]:
    """Run complete benchmark: compile O0/O2 + execute both."""
    print(f"\n{'='*70}")
    print(f"Benchmark: {name}")
    print(f"Source: {graph_file}")
    if use_mock:
        print(f"Mode: MOCK (latency={mock_latency}ms)")
    else:
        print(f"Mode: REAL LLM")
    print(f"{'='*70}")

    if not graph_file.exists():
        print(f"ERROR: Source file not found: {graph_file}")
        return {}

    # If it's a Python file, generate .apxm first
    if graph_file.suffix == ".py":
        apxm_file = generate_graph_from_python(graph_file)
        if not apxm_file:
            return {}
        graph_file = apxm_file

    results = {}

    # Compile and execute at O0 and O2
    for opt_level in [0, 2]:
        print(f"\nConfiguration: O{opt_level}")

            # Compile
        compile_metrics = compile_and_measure(graph_file, opt_level, use_mock=use_mock)
        if not compile_metrics:
            continue

        # Execute
        artifact_file = graph_file.with_suffix(f".O{opt_level}.apxmobj")
        execution_metrics = execute_and_measure(
            artifact_file,
            use_mock=use_mock,
            mock_latency=mock_latency,
        )
        if not execution_metrics:
            continue

        results[f"O{opt_level}"] = BenchmarkResult(
            opt_level=opt_level,
            compile=compile_metrics,
            execution=execution_metrics,
        )

    return results


def format_results_table(
    benchmark_name: str,
    results: Dict[str, BenchmarkResult],
    config: Dict,
) -> str:
    """Format benchmark results as markdown table."""
    if not results or len(results) < 2:
        return f"\n## {benchmark_name}\n\nInsufficient data (need both O0 and O2)\n"

    o0 = results.get("O0")
    o2 = results.get("O2")

    if not o0 or not o2:
        return f"\n## {benchmark_name}\n\nMissing O0 or O2 results\n"

    lines = []
    lines.append(f"\n## {benchmark_name}")
    lines.append(f"**Description**: {config['description']}")
    lines.append(f"**Expected**: {config.get('expected_improvement', 'N/A')}")
    lines.append("")

    # Compilation Metrics
    lines.append("### Compilation Phase")
    lines.append("")
    lines.append("| Metric | O0 | O2 | Improvement |")
    lines.append("|--------|----|----|-------------|")

    comp_metrics = [
        ("Nodes", o0.compile.nodes_after, o2.compile.nodes_after),
        ("Est. Tokens", o0.compile.tokens_before, o2.compile.tokens_after),
        ("Artifact Size (bytes)", o0.compile.artifact_size, o2.compile.artifact_size),
        ("Compile Time (ms)", f"{o0.compile.compile_time_ms:.0f}", f"{o2.compile.compile_time_ms:.0f}"),
    ]

    for name, v0, v2 in comp_metrics:
        if isinstance(v0, (int, float)) and isinstance(v2, (int, float)):
            if v0 > 0:
                improvement = ((v0 - v2) / v0) * 100
                lines.append(f"| {name} | {v0} | {v2} | {improvement:+.1f}% |")
            else:
                lines.append(f"| {name} | {v0} | {v2} | N/A |")
        else:
            lines.append(f"| {name} | {v0} | {v2} | N/A |")

    if o2.compile.optimizations_applied:
        lines.append(f"| Optimizations | - | {', '.join(o2.compile.optimizations_applied)} | - |")

    lines.append("")

    # Execution Metrics
    lines.append("### Execution Phase")
    lines.append("")
    lines.append("| Metric | O0 | O2 | Improvement |")
    lines.append("|--------|----|----|-------------|")

    exec_metrics = [
        ("Wall Time (ms)", o0.execution.wall_time_ms, o2.execution.wall_time_ms),
        ("Execution Time (ms)", o0.execution.execution_time_ms, o2.execution.execution_time_ms),
        ("Nodes Executed", o0.execution.nodes_executed, o2.execution.nodes_executed),
        ("LLM Calls", o0.execution.llm_calls, o2.execution.llm_calls),
        ("Input Tokens", o0.execution.input_tokens, o2.execution.input_tokens),
        ("Output Tokens", o0.execution.output_tokens, o2.execution.output_tokens),
        ("Cache Hits", o0.execution.cache_hits, o2.execution.cache_hits),
        ("Avg Parallelism", f"{o0.execution.avg_parallelism:.2f}", f"{o2.execution.avg_parallelism:.2f}"),
        ("Max Parallelism", o0.execution.max_parallelism, o2.execution.max_parallelism),
    ]

    for name, v0, v2 in exec_metrics:
        if isinstance(v0, (int, float)) and isinstance(v2, (int, float)):
            if v0 > 0:
                improvement = ((v0 - v2) / v0) * 100
                lines.append(f"| {name} | {v0} | {v2} | {improvement:+.1f}% |")
            else:
                lines.append(f"| {name} | {v0} | {v2} | N/A |")
        else:
            lines.append(f"| {name} | {v0} | {v2} | - |")

    lines.append("")

    # Overall Speedup
    if o0.execution.wall_time_ms > 0 and o2.execution.wall_time_ms > 0:
        speedup = o0.execution.wall_time_ms / o2.execution.wall_time_ms
        lines.append(f"**Overall Speedup**: {speedup:.2f}x")
        lines.append("")

    return "\n".join(lines)


def format_summary_table(all_results: Dict[str, Dict[str, BenchmarkResult]]) -> str:
    """Format summary comparison across all benchmarks."""
    lines = []
    lines.append("\n" + "=" * 80)
    lines.append("SUMMARY: O0 vs O2 Comparison")
    lines.append("=" * 80)
    lines.append("")
    lines.append("| Benchmark | Nodes (O0→O2) | Tokens (O0→O2) | Wall Time (O0→O2) | Speedup |")
    lines.append("|-----------|---------------|----------------|-------------------|---------|")

    for bench_name, results in all_results.items():
        if "O0" not in results or "O2" not in results:
            continue

        o0 = results["O0"]
        o2 = results["O2"]

        nodes_str = f"{o0.compile.nodes_after} → {o2.compile.nodes_after}"
        tokens_str = f"{o0.compile.tokens_before} → {o2.compile.tokens_after}"
        time_str = f"{o0.execution.wall_time_ms:.0f}ms → {o2.execution.wall_time_ms:.0f}ms"

        speedup = 1.0
        if o0.execution.wall_time_ms > 0 and o2.execution.wall_time_ms > 0:
            speedup = o0.execution.wall_time_ms / o2.execution.wall_time_ms

        speedup_str = f"{speedup:.2f}x"

        lines.append(f"| {bench_name} | {nodes_str} | {tokens_str} | {time_str} | {speedup_str} |")

    lines.append("")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(
        description="End-to-end benchmark: compiler + runtime optimizations",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )

    parser.add_argument(
        "--all",
        action="store_true",
        help="Run all benchmarks",
    )
    parser.add_argument(
        "--graph",
        choices=list(BENCHMARKS.keys()),
        help="Run specific benchmark",
    )
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Use mock LLM backend (reproducible)",
    )
    parser.add_argument(
        "--real",
        action="store_true",
        help="Use real LLM backend (overrides --mock)",
    )
    parser.add_argument(
        "--mock-latency",
        type=int,
        default=500,
        help="Mock backend latency in ms (default: 500)",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="List available benchmarks",
    )
    parser.add_argument(
        "-o", "--output",
        type=Path,
        help="Output file for markdown report (default: stdout)",
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

    # Determine mock vs real
    use_mock = args.mock
    if args.real:
        use_mock = False

    # Run benchmarks
    all_results = {}

    for bench_name in benchmarks_to_run:
        config = BENCHMARKS[bench_name]
        graph_file = PROJECT_ROOT / config["file"]

        results = run_benchmark(
            bench_name,
            graph_file,
            use_mock=use_mock,
            mock_latency=args.mock_latency,
        )

        if results:
            all_results[bench_name] = results

    # Format report
    report_lines = []
    report_lines.append("=" * 80)
    report_lines.append("END-TO-END BENCHMARK RESULTS")
    report_lines.append("Compiler + Runtime Optimization Impact")
    report_lines.append("=" * 80)
    report_lines.append(f"\nGenerated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    report_lines.append(f"Mode: {'MOCK' if use_mock else 'REAL LLM'}")
    if use_mock:
        report_lines.append(f"Mock Latency: {args.mock_latency}ms")
    report_lines.append("")

    # Individual benchmark details
    for bench_name, results in all_results.items():
        config = BENCHMARKS[bench_name]
        report_lines.append(format_results_table(bench_name, results, config))

    # Summary table
    if len(all_results) > 1:
        report_lines.append(format_summary_table(all_results))

    report = "\n".join(report_lines)

    # Output report
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with open(args.output, "w") as f:
            f.write(report)
        print(f"\n{'='*70}")
        print(f"Report written to: {args.output}")
    else:
        print(report)

    # Save raw JSON
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    json_file = PROJECT_ROOT / f"benchmark_e2e_{timestamp}.json"

    # Convert dataclass results to dict for JSON serialization
    json_results = {}
    for bench_name, results in all_results.items():
        json_results[bench_name] = {}
        for opt_key, result in results.items():
            json_results[bench_name][opt_key] = {
                "opt_level": result.opt_level,
                "compile": {
                    "nodes_before": result.compile.nodes_before,
                    "nodes_after": result.compile.nodes_after,
                    "tokens_before": result.compile.tokens_before,
                    "tokens_after": result.compile.tokens_after,
                    "artifact_size": result.compile.artifact_size,
                    "compile_time_ms": result.compile.compile_time_ms,
                    "optimizations_applied": result.compile.optimizations_applied,
                },
                "execution": {
                    "wall_time_ms": result.execution.wall_time_ms,
                    "execution_time_ms": result.execution.execution_time_ms,
                    "nodes_executed": result.execution.nodes_executed,
                    "nodes_failed": result.execution.nodes_failed,
                    "llm_calls": result.execution.llm_calls,
                    "input_tokens": result.execution.input_tokens,
                    "output_tokens": result.execution.output_tokens,
                    "cache_hits": result.execution.cache_hits,
                    "avg_parallelism": result.execution.avg_parallelism,
                    "max_parallelism": result.execution.max_parallelism,
                    "critical_path_ms": result.execution.critical_path_ms,
                },
            }

    with open(json_file, "w") as f:
        json.dump(json_results, f, indent=2)

    print(f"Raw JSON saved to: {json_file}")
    print()

    return 0


if __name__ == "__main__":
    sys.exit(main())
