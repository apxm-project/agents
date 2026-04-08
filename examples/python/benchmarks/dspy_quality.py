#!/usr/bin/env python3
"""dspy_quality.py - Benchmark DSPy optimization quality

Tests: Compare LLM template quality with and without DSPy optimization
Measures: Template structure changes, few-shot examples added, instruction enhancement

This benchmark:
1. Builds a workflow with 3 LLM operations
2. Runs it without DSPy optimization (baseline)
3. Optimizes templates using DSPy with training data
4. Compares the before/after template structure
5. Optionally runs with real LLM to measure output quality

Usage:
  # Generate both baseline and optimized graphs
  python3 examples/python/benchmarks/dspy_quality.py

  # Execute baseline
  dekk apxm execute /tmp/dspy_quality_baseline.apxm -O0

  # Execute optimized
  dekk apxm execute /tmp/dspy_quality_optimized.apxm -O0

  # Compare results
  python3 examples/python/benchmarks/dspy_quality.py --compare
"""

import json
import logging
import sys
from pathlib import Path
from typing import Dict, Any, List

from apxm import compile, GraphRecorder
from apxm.dspy_bridge import ApxmDspyBridge, OptimizationConfig, load_training_data

logging.basicConfig(level=logging.INFO, format="%(levelname)s: %(message)s")


@compile()
def technical_qa_pipeline(g: GraphRecorder):
    """Three-stage LLM pipeline for technical Q&A.

    This workflow demonstrates DSPy optimization across multiple LLM operations:
    - ASK: Initial technical question answering
    - THINK: Deep analysis of the topic
    - REASON: Synthesis with related concepts
    """

    # Stage 1: Direct answer
    answer = g.ask(
        "answer",
        "{{question}}"
    )

    # Stage 2: Deep analysis
    analysis = g.think(
        "analysis",
        "Given this answer:\n{{answer}}\n\nProvide a detailed technical analysis covering:\n"
        "1. Core concepts involved\n"
        "2. Common misconceptions\n"
        "3. Real-world applications\n"
        "4. Related technologies or patterns"
    )

    # Stage 3: Synthesis and connections
    synthesis = g.reason(
        "synthesis",
        "Based on this analysis:\n{{analysis}}\n\nSynthesize the key insights and explain how "
        "this concept connects to broader software engineering principles. "
        "What are the implications for system design?"
    )

    # Format output
    output = g.print(
        "=== Technical Q&A Results ===\n\n"
        "Question: {{question}}\n\n"
        "Answer:\n{{answer}}\n\n"
        "---\n\n"
        "Analysis:\n{{analysis}}\n\n"
        "---\n\n"
        "Synthesis:\n{{synthesis}}\n"
    )

    g.done(output)


def analyze_template_changes(
    baseline_graph: Dict[str, Any],
    optimized_graph: Dict[str, Any]
) -> Dict[str, Any]:
    """Compare baseline and optimized graph templates."""

    results = {
        "nodes_analyzed": 0,
        "nodes_optimized": 0,
        "changes": []
    }

    # Build node lookup by name
    baseline_nodes = {n["name"]: n for n in baseline_graph.get("nodes", [])}
    optimized_nodes = {n["name"]: n for n in optimized_graph.get("nodes", [])}

    llm_ops = {"ASK", "THINK", "REASON"}

    for name, baseline_node in baseline_nodes.items():
        if baseline_node.get("op") not in llm_ops:
            continue

        results["nodes_analyzed"] += 1

        optimized_node = optimized_nodes.get(name)
        if not optimized_node:
            continue

        baseline_attrs = baseline_node.get("attributes", {})
        optimized_attrs = optimized_node.get("attributes", {})

        baseline_template = baseline_attrs.get("template_str", "")
        optimized_template = optimized_attrs.get("__dspy_optimized_template", "")

        if optimized_template and optimized_template != baseline_template:
            results["nodes_optimized"] += 1

            # Analyze changes
            change = {
                "node_name": name,
                "op": baseline_node.get("op"),
                "baseline_length": len(baseline_template),
                "optimized_length": len(optimized_template),
                "length_increase": len(optimized_template) - len(baseline_template),
                "has_examples": "Examples:" in optimized_template,
                "example_count": optimized_template.count("Example "),
                "has_instructions": len(optimized_template.split("\n")[0]) > len(baseline_template.split("\n")[0]),
                "baseline_template": baseline_template[:200] + "..." if len(baseline_template) > 200 else baseline_template,
                "optimized_template": optimized_template[:200] + "..." if len(optimized_template) > 200 else optimized_template,
            }
            results["changes"].append(change)

    return results


def print_comparison_report(analysis: Dict[str, Any]):
    """Print a formatted comparison report."""

    print("\n" + "=" * 80)
    print("DSPy Optimization Quality Report")
    print("=" * 80)
    print(f"\nNodes analyzed: {analysis['nodes_analyzed']}")
    print(f"Nodes optimized: {analysis['nodes_optimized']}")
    print(f"Optimization rate: {analysis['nodes_optimized'] / max(1, analysis['nodes_analyzed']) * 100:.1f}%")

    if not analysis['changes']:
        print("\nNo template changes detected.")
        return

    print("\n" + "-" * 80)
    print("Template Changes by Node:")
    print("-" * 80)

    for change in analysis['changes']:
        print(f"\nNode: {change['node_name']} ({change['op']})")
        print(f"  Template length: {change['baseline_length']} → {change['optimized_length']} "
              f"({change['length_increase']:+d} chars)")
        print(f"  Few-shot examples: {'Yes' if change['has_examples'] else 'No'}")
        if change['has_examples']:
            print(f"  Example count: {change['example_count']}")
        print(f"  Enhanced instructions: {'Yes' if change['has_instructions'] else 'No'}")

        print(f"\n  Baseline template (first 200 chars):")
        print(f"    {change['baseline_template']}")
        print(f"\n  Optimized template (first 200 chars):")
        print(f"    {change['optimized_template']}")

    # Summary statistics
    print("\n" + "-" * 80)
    print("Summary Statistics:")
    print("-" * 80)

    total_length_increase = sum(c['length_increase'] for c in analysis['changes'])
    avg_length_increase = total_length_increase / len(analysis['changes'])
    nodes_with_examples = sum(1 for c in analysis['changes'] if c['has_examples'])
    total_examples = sum(c['example_count'] for c in analysis['changes'])

    print(f"  Total template length increase: {total_length_increase:+d} chars")
    print(f"  Average length increase per node: {avg_length_increase:+.1f} chars")
    print(f"  Nodes with few-shot examples: {nodes_with_examples}/{len(analysis['changes'])}")
    print(f"  Total few-shot examples added: {total_examples}")

    print("\n" + "=" * 80 + "\n")


def main():
    """Run DSPy quality benchmark."""

    import argparse
    parser = argparse.ArgumentParser(description="DSPy optimization quality benchmark")
    parser.add_argument(
        "--compare",
        action="store_true",
        help="Compare previously generated baseline and optimized graphs"
    )
    parser.add_argument(
        "--optimizer",
        default="labeled_fewshot",
        choices=["labeled_fewshot", "bootstrap_fewshot"],
        help="DSPy optimizer to use (default: labeled_fewshot, no API key needed)"
    )
    parser.add_argument(
        "--max-demos",
        type=int,
        default=3,
        help="Maximum few-shot demos to include (default: 3)"
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Verbose logging"
    )

    args = parser.parse_args()

    if args.verbose:
        logging.getLogger().setLevel(logging.DEBUG)

    baseline_path = Path("/tmp/dspy_quality_baseline.apxm")
    optimized_path = Path("/tmp/dspy_quality_optimized.apxm")

    if args.compare:
        # Load and compare existing graphs
        if not baseline_path.exists() or not optimized_path.exists():
            print("Error: Baseline or optimized graph not found. Run without --compare first.")
            return 1

        with open(baseline_path) as f:
            baseline_graph = json.load(f)
        with open(optimized_path) as f:
            optimized_graph = json.load(f)

        analysis = analyze_template_changes(baseline_graph, optimized_graph)
        print_comparison_report(analysis)

        return 0

    # Generate baseline graph
    print("Generating baseline graph...")
    baseline_graph = technical_qa_pipeline._graph.to_dict()

    with open(baseline_path, "w") as f:
        json.dump(baseline_graph, f, indent=2)
    print(f"✓ Baseline graph written to {baseline_path}")

    # Load training data
    training_data_path = Path(__file__).parent / "dspy_training_data.json"
    if not training_data_path.exists():
        print(f"Error: Training data not found at {training_data_path}")
        return 1

    training_data = load_training_data(training_data_path)
    print(f"✓ Loaded {len(training_data)} training examples")

    # Optimize with DSPy
    print(f"\nOptimizing with DSPy ({args.optimizer})...")
    config = OptimizationConfig(
        optimizer=args.optimizer,
        max_labeled_demos=args.max_demos,
        max_bootstrapped_demos=args.max_demos,
        verbose=args.verbose,
        fallback_to_labeled=True,
    )

    bridge = ApxmDspyBridge()
    optimized_graph = bridge.optimize_graph(baseline_graph, training_data, config)

    # Save optimized graph
    if hasattr(optimized_graph, 'to_dict'):
        optimized_dict = optimized_graph.to_dict()
    else:
        optimized_dict = optimized_graph

    with open(optimized_path, "w") as f:
        json.dump(optimized_dict, f, indent=2)
    print(f"✓ Optimized graph written to {optimized_path}")

    # Analyze and report changes
    print("\nAnalyzing template changes...")
    analysis = analyze_template_changes(baseline_graph, optimized_dict)
    print_comparison_report(analysis)

    # Execution instructions
    print("\nNext steps:")
    print(f"  1. Execute baseline:  dekk apxm execute {baseline_path}")
    print(f"  2. Execute optimized: dekk apxm execute {optimized_path}")
    print(f"  3. Compare outputs to measure quality improvement")
    print(f"  4. Run with --compare flag to regenerate this report\n")

    return 0


if __name__ == "__main__":
    sys.exit(main())
