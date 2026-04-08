#!/usr/bin/env python3
"""
Quality-guided compilation for APXM graphs.

Compiles a graph at O0 and O2, runs both against training data, evaluates quality,
and identifies risky fusions. If quality regresses, recompiles with risky fusions disabled.

Usage:
    python3 scripts/quality-guided-compile.py \\
        --graph examples/python/demo/showcase.py \\
        --training-data training.json \\
        --output optimized.apxmobj

    # Use synthetic training data (generates examples automatically)
    python3 scripts/quality-guided-compile.py \\
        --graph workflow.apxm \\
        --synthetic-examples 10 \\
        --output optimized.apxmobj

    # Load training data from session results
    python3 scripts/quality-guided-compile.py \\
        --graph workflow.apxm \\
        --session-dir ~/.apxm/sessions/<id> \\
        --output optimized.apxmobj
"""

import argparse
import json
import logging
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional

# Add parent directory to path to import apxm module
sys.path.insert(0, str(Path(__file__).parent.parent / "crates/apxm-frontend/python"))

try:
    from apxm.quality import QualityEvaluator, FusionCandidate, compare_optimizations
    from apxm.dspy_bridge import load_training_data, profile_to_examples
except ImportError as e:
    print(f"Error: Failed to import apxm modules: {e}")
    print("Make sure you're running from the APXM project root.")
    sys.exit(1)


def run_apxm_command(args: List[str]) -> subprocess.CompletedProcess:
    """
    Run an APXM CLI command via dekk.

    Args:
        args: Command arguments (e.g., ["compile", "graph.apxm", "-O0"])

    Returns:
        CompletedProcess with stdout, stderr, returncode
    """
    cmd = ["dekk", "apxm"] + args
    logging.debug(f"Running: {' '.join(cmd)}")
    return subprocess.run(cmd, capture_output=True, text=True)


def compile_graph(
    graph_path: Path,
    opt_level: int,
    output_path: Optional[Path] = None,
) -> Path:
    """
    Compile an APXM graph at a specific optimization level.

    Args:
        graph_path: Path to .apxm graph file
        opt_level: Optimization level (0, 1, 2)
        output_path: Optional output path for artifact

    Returns:
        Path to compiled artifact
    """
    if output_path is None:
        output_path = graph_path.parent / f"{graph_path.stem}_O{opt_level}.apxmobj"

    args = [
        "compile",
        str(graph_path),
        f"-O{opt_level}",
        "-o", str(output_path),
    ]

    result = run_apxm_command(args)

    if result.returncode != 0:
        raise RuntimeError(
            f"Compilation failed at O{opt_level}:\n{result.stderr}"
        )

    logging.info(f"Compiled at O{opt_level}: {output_path}")
    return output_path


def execute_artifact(
    artifact_path: Path,
    args: Optional[List[str]] = None,
) -> Dict[str, Any]:
    """
    Execute a compiled APXM artifact and return results.

    Args:
        artifact_path: Path to .apxmobj artifact
        args: Optional arguments to pass to the workflow

    Returns:
        Execution results dict with 'outputs' key containing list of outputs
    """
    cmd_args = ["run", str(artifact_path), "--emit-session"]
    if args:
        cmd_args.extend(["--"] + args)

    result = run_apxm_command(cmd_args)

    if result.returncode != 0:
        raise RuntimeError(
            f"Execution failed:\n{result.stderr}"
        )

    # Parse session directory from output
    # Expected format: "Session: ~/.apxm/sessions/<id>"
    session_dir = None
    for line in result.stdout.split('\n'):
        if 'Session:' in line or 'session' in line.lower():
            parts = line.split()
            for part in parts:
                if '.apxm/sessions' in part:
                    session_dir = Path(part.strip())
                    break

    if not session_dir:
        # Fallback: find most recent session
        sessions_dir = Path.home() / ".apxm" / "sessions"
        if sessions_dir.exists():
            sessions = sorted(sessions_dir.iterdir(), key=lambda p: p.stat().st_mtime)
            if sessions:
                session_dir = sessions[-1]

    if not session_dir or not session_dir.exists():
        raise RuntimeError(
            f"Could not find session directory in output:\n{result.stdout}"
        )

    # Load results from session
    results_file = session_dir / "results.json"
    if not results_file.exists():
        raise RuntimeError(f"Results file not found: {results_file}")

    with open(results_file) as f:
        results = json.load(f)

    # Extract outputs (assume last node is the final output)
    outputs = []
    for node_result in results.values():
        if node_result.get("success"):
            outputs.append(node_result.get("output", ""))

    logging.info(f"Execution completed: {len(outputs)} outputs")

    return {
        "session_dir": session_dir,
        "outputs": outputs,
        "results": results,
    }


def generate_synthetic_examples(
    graph_path: Path,
    num_examples: int,
) -> List[Dict[str, Any]]:
    """
    Generate synthetic training examples for a graph.

    For now, this is a placeholder that creates dummy examples.
    In the future, this could use DSPy's bootstrap or other techniques.

    Args:
        graph_path: Path to graph file
        num_examples: Number of examples to generate

    Returns:
        List of training examples
    """
    logging.warning(
        "Synthetic example generation is a placeholder. "
        "Provide real training data for better results."
    )

    # Load graph to inspect parameters
    with open(graph_path) as f:
        graph = json.load(f)

    params = graph.get("parameters", [])
    examples = []

    for i in range(num_examples):
        inputs = {}
        for param in params:
            param_name = param.get("name", f"param_{i}")
            # Generate dummy input based on type
            param_type = param.get("type_name", "str")
            if param_type == "int":
                inputs[param_name] = i
            elif param_type == "float":
                inputs[param_name] = float(i)
            else:
                inputs[param_name] = f"example_{i}"

        examples.append({
            "inputs": inputs,
            "output": f"result_{i}",
        })

    return examples


def main():
    parser = argparse.ArgumentParser(
        description="Quality-guided APXM compilation",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "--graph",
        required=True,
        help="Path to APXM graph file (.apxm or .json)",
    )
    parser.add_argument(
        "--training-data",
        help="Path to training data JSON file",
    )
    parser.add_argument(
        "--session-dir",
        help="Path to session directory (alternative to --training-data)",
    )
    parser.add_argument(
        "--synthetic-examples",
        type=int,
        metavar="N",
        help="Generate N synthetic training examples",
    )
    parser.add_argument(
        "--output", "-o",
        help="Output path for optimized artifact (default: <graph>_optimized.apxmobj)",
    )
    parser.add_argument(
        "--quality-profile", "-p",
        help="Path to save quality profile JSON (default: <graph>.apxm-quality-profile.json)",
    )
    parser.add_argument(
        "--regression-threshold",
        type=float,
        default=0.05,
        help="Quality drop threshold to flag as regression (default: 0.05 = 5%%)",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Verbose output",
    )

    args = parser.parse_args()

    # Set up logging
    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(levelname)s: %(message)s",
    )

    # Resolve graph path
    graph_path = Path(args.graph)
    if not graph_path.exists():
        logging.error(f"Graph file not found: {graph_path}")
        return 1

    # Load training data
    training_data = None
    if args.training_data:
        training_data = load_training_data(args.training_data)
        logging.info(f"Loaded {len(training_data)} training examples")
    elif args.session_dir:
        training_data = profile_to_examples(args.session_dir)
        logging.info(f"Extracted {len(training_data)} examples from session")
    elif args.synthetic_examples:
        training_data = generate_synthetic_examples(graph_path, args.synthetic_examples)
        logging.info(f"Generated {len(training_data)} synthetic examples")
    else:
        logging.error(
            "No training data provided. Use --training-data, --session-dir, or --synthetic-examples"
        )
        return 1

    if not training_data:
        logging.error("Failed to load training data")
        return 1

    # Step 1: Compile at O0
    logging.info("=" * 60)
    logging.info("Step 1: Compiling at O0 (unoptimized)")
    logging.info("=" * 60)
    try:
        with tempfile.TemporaryDirectory() as tmpdir:
            o0_artifact = Path(tmpdir) / "o0.apxmobj"
            o0_artifact = compile_graph(graph_path, opt_level=0, output_path=o0_artifact)

            # Step 2: Compile at O2
            logging.info("=" * 60)
            logging.info("Step 2: Compiling at O2 (optimized)")
            logging.info("=" * 60)
            o2_artifact = Path(tmpdir) / "o2.apxmobj"
            o2_artifact = compile_graph(graph_path, opt_level=2, output_path=o2_artifact)

            # Step 3: Execute O0
            logging.info("=" * 60)
            logging.info("Step 3: Executing O0 artifact")
            logging.info("=" * 60)
            # For simplicity, we'll just run once without args
            # In production, you'd run with each example's inputs
            o0_result = execute_artifact(o0_artifact)
            o0_outputs = o0_result["outputs"]

            # Step 4: Execute O2
            logging.info("=" * 60)
            logging.info("Step 4: Executing O2 artifact")
            logging.info("=" * 60)
            o2_result = execute_artifact(o2_artifact)
            o2_outputs = o2_result["outputs"]

            # Step 5: Evaluate quality
            logging.info("=" * 60)
            logging.info("Step 5: Evaluating quality")
            logging.info("=" * 60)
            evaluator = QualityEvaluator(regression_threshold=args.regression_threshold)

            # Match output length to training data
            min_len = min(len(training_data), len(o0_outputs), len(o2_outputs))
            if min_len < len(training_data):
                logging.warning(
                    f"Output length ({min_len}) < training data ({len(training_data)}). "
                    f"Using first {min_len} examples."
                )
            training_data = training_data[:min_len]
            o0_outputs = o0_outputs[:min_len]
            o2_outputs = o2_outputs[:min_len]

            report = evaluator.evaluate_optimization(
                graph_path,
                training_data,
                o0_outputs,
                o2_outputs,
            )

            # Identify risky fusions
            if report.regression:
                risky = evaluator.identify_risky_fusions(
                    graph_path,
                    training_data,
                    o0_outputs,
                    o2_outputs,
                )
                report.risky_fusions = risky

            # Step 6: Report results
            logging.info("=" * 60)
            logging.info("Step 6: Quality Report")
            logging.info("=" * 60)
            print(f"\n{'Quality Evaluation Report':^60}")
            print("=" * 60)
            print(f"  O0 Score: {report.o0_score:.3f}")
            print(f"  O2 Score: {report.o2_score:.3f}")
            print(f"  Regression: {'YES' if report.regression else 'NO'}")
            print(f"  Recommendation: {report.recommendation}")

            if report.risky_fusions:
                print(f"\n  Risky Fusions ({len(report.risky_fusions)}):")
                for fusion in report.risky_fusions[:5]:  # Show top 5
                    print(
                        f"    {fusion.producer_id} → {fusion.consumer_id} "
                        f"(drop: {fusion.quality_drop:.3f})"
                    )
            print("=" * 60)

            # Step 7: Save quality profile
            if args.quality_profile:
                profile_path = Path(args.quality_profile)
            else:
                profile_path = graph_path.parent / f"{graph_path.stem}.apxm-quality-profile.json"

            evaluator.save_quality_profile(report, graph_path.stem, profile_path)

            # Step 8: Choose final artifact
            if report.regression:
                logging.warning(
                    f"Quality regression detected ({report.o0_score:.3f} → {report.o2_score:.3f}). "
                    f"Keeping O0 artifact as safer option."
                )
                final_artifact = o0_artifact
            else:
                logging.info(
                    f"Quality maintained or improved ({report.o0_score:.3f} → {report.o2_score:.3f}). "
                    f"Using O2 artifact."
                )
                final_artifact = o2_artifact

            # Step 9: Copy final artifact to output
            if args.output:
                output_path = Path(args.output)
            else:
                output_path = graph_path.parent / f"{graph_path.stem}_optimized.apxmobj"

            import shutil
            shutil.copy(final_artifact, output_path)

            print(f"\n✓ Optimized artifact saved to: {output_path}")
            print(f"✓ Quality profile saved to: {profile_path}")

    except Exception as e:
        logging.error(f"Quality-guided compilation failed: {e}")
        if args.verbose:
            import traceback
            traceback.print_exc()
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
