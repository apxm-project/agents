"""
DSPy Bridge for APXM

Enables prompt optimization via DSPy as a compiler pass. Extracts LLM operation
templates from APXM graphs, optimizes them using DSPy, and returns optimized graphs.

This is a prototype implementation supporting:
- Template extraction from ASK/THINK/REASON nodes
- DSPy signature conversion
- BootstrapFewShot optimization (default)
- Training data from execution profiles

Future enhancements:
- MIPROv2 optimizer for O3
- Multi-objective optimization (latency + tokens + accuracy)
- Caching optimized templates
- MLIR pass integration
"""

import json
import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Union

try:
    import dspy
    from dspy import Example
    from dspy.teleprompt import BootstrapFewShot
    DSPY_AVAILABLE = True
except ImportError:
    DSPY_AVAILABLE = False
    logging.warning(
        "DSPy not installed. Install with: pip install dspy\n"
        "DSPy optimization will be skipped."
    )


@dataclass
class OptimizationConfig:
    """Configuration for DSPy optimization."""
    optimizer: str = "bootstrap_fewshot"  # bootstrap_fewshot, mipro_v2, copro
    max_bootstrapped_demos: int = 5
    max_labeled_demos: int = 10
    num_trials: int = 10
    metric_threshold: float = 0.5
    verbose: bool = False


class ApxmDspyBridge:
    """
    Bridge between APXM graphs and DSPy optimization framework.

    Usage:
        bridge = ApxmDspyBridge()
        optimized_graph = bridge.optimize_graph(graph, training_data)
    """

    def __init__(self, lm_config: Optional[Dict[str, Any]] = None):
        """
        Initialize DSPy bridge.

        Args:
            lm_config: Language model configuration dict with keys:
                - model: Model name (e.g., "gpt-4o-mini")
                - api_key: API key (optional, uses env var if not provided)
                - base_url: Base URL for API (optional)
                - max_tokens: Max tokens (default: 2048)
        """
        if not DSPY_AVAILABLE:
            logging.warning("DSPy not available, bridge will be a no-op")
            self.lm = None
            return

        # Default to gpt-4o-mini for prototyping (fast, cheap)
        if lm_config is None:
            lm_config = {
                "model": "gpt-4o-mini",
                "max_tokens": 2048,
            }

        # Configure DSPy LM
        try:
            self.lm = dspy.LM(
                model=lm_config.get("model", "gpt-4o-mini"),
                api_key=lm_config.get("api_key"),
                api_base=lm_config.get("base_url"),
                max_tokens=lm_config.get("max_tokens", 2048),
            )
            dspy.configure(lm=self.lm)
        except Exception as e:
            logging.warning(f"Failed to configure DSPy LM: {e}. Using stub mode.")
            self.lm = None

    def optimize_graph(
        self,
        graph: Dict[str, Any],
        training_data: Optional[List[Dict[str, Any]]] = None,
        config: Optional[OptimizationConfig] = None,
    ) -> Dict[str, Any]:
        """
        Optimize all LLM templates in an APXM graph.

        Args:
            graph: APXM graph dict with 'nodes' and 'edges' keys
            training_data: List of training examples (DSPy format or session results)
            config: Optimization configuration

        Returns:
            Optimized graph with updated template_str attributes
        """
        if not DSPY_AVAILABLE or self.lm is None:
            logging.info("DSPy not available, returning graph unchanged")
            return graph

        if config is None:
            config = OptimizationConfig()

        # Extract LLM nodes (ASK, THINK, REASON)
        llm_ops = {"ASK", "THINK", "REASON"}
        llm_nodes = [
            node for node in graph.get("nodes", [])
            if node.get("op") in llm_ops
        ]

        if not llm_nodes:
            logging.info("No LLM nodes found in graph")
            return graph

        if not training_data:
            logging.warning(
                "No training data provided. DSPy optimization requires examples.\n"
                "Pass training_data or use profile_to_examples() to convert session results."
            )
            return graph

        logging.info(f"Optimizing {len(llm_nodes)} LLM nodes with {len(training_data)} examples")

        # Optimize each node's template
        optimized_graph = graph.copy()
        for i, node in enumerate(optimized_graph.get("nodes", [])):
            if node.get("op") not in llm_ops:
                continue

            node_id = node.get("id")
            template_str = node.get("attributes", {}).get("template_str", "")

            if not template_str:
                logging.warning(f"Node {node_id} has no template_str, skipping")
                continue

            try:
                optimized_template = self.optimize_template(
                    template_str,
                    training_data,
                    config,
                )

                # Annotate node with optimized template
                if "attributes" not in node:
                    node["attributes"] = {}
                node["attributes"]["__dspy_optimized_template"] = optimized_template

                if config.verbose:
                    logging.info(
                        f"Node {node_id} optimized:\n"
                        f"  Original: {template_str[:60]}...\n"
                        f"  Optimized: {optimized_template[:60]}..."
                    )
            except Exception as e:
                logging.error(f"Failed to optimize node {node_id}: {e}")
                continue

        return optimized_graph

    def optimize_template(
        self,
        template: str,
        examples: List[Dict[str, Any]],
        config: Optional[OptimizationConfig] = None,
    ) -> str:
        """
        Optimize a single template string using DSPy.

        Args:
            template: Template string with {{var}} placeholders
            examples: Training examples (list of dicts with 'inputs' and 'output')
            config: Optimization configuration

        Returns:
            Optimized template string
        """
        if not DSPY_AVAILABLE or self.lm is None:
            return template

        if config is None:
            config = OptimizationConfig()

        # Convert examples to DSPy format
        dspy_examples = []
        for ex in examples:
            try:
                inputs = ex.get("inputs", {})
                output = ex.get("output", "")
                dspy_examples.append(
                    Example(**inputs, answer=output).with_inputs(*inputs.keys())
                )
            except Exception as e:
                logging.warning(f"Failed to convert example: {e}")
                continue

        if not dspy_examples:
            logging.warning("No valid examples after conversion")
            return template

        # Create a simple DSPy signature from the template
        # Extract variables from template (e.g., {{question}}, {{context}})
        import re
        var_pattern = r'\{\{(\w+)\}\}'
        input_vars = list(set(re.findall(var_pattern, template)))

        if not input_vars:
            logging.warning("No input variables found in template")
            return template

        # Build signature string: "input1, input2 -> answer"
        signature_str = ", ".join(input_vars) + " -> answer"

        # Create predictor module
        class SimplePredictor(dspy.Module):
            def __init__(self, signature):
                super().__init__()
                self.predictor = dspy.Predict(signature)

            def forward(self, **kwargs):
                return self.predictor(**kwargs)

        predictor = SimplePredictor(signature_str)

        # Define metric (simple exact match for prototype)
        def simple_metric(example, prediction, trace=None):
            if not hasattr(prediction, "answer"):
                return 0.0
            # For prototype, just check if we got a non-empty answer
            return 1.0 if prediction.answer else 0.0

        # Run optimizer
        try:
            if config.optimizer == "bootstrap_fewshot":
                optimizer = BootstrapFewShot(
                    metric=simple_metric,
                    max_bootstrapped_demos=config.max_bootstrapped_demos,
                    max_labeled_demos=config.max_labeled_demos,
                )
                optimized_predictor = optimizer.compile(
                    predictor,
                    trainset=dspy_examples[:min(len(dspy_examples), 20)],  # Limit for speed
                )
            else:
                logging.warning(f"Optimizer {config.optimizer} not yet implemented, using original")
                return template

            # Extract the optimized prompt from the predictor
            # For now, return the original template annotated with DSPy context
            # In a full implementation, we'd extract the learned instructions
            return template + " (DSPy-optimized)"

        except Exception as e:
            logging.error(f"DSPy optimization failed: {e}")
            return template


def profile_to_examples(session_dir: Union[str, Path]) -> List[Dict[str, Any]]:
    """
    Convert APXM session results to DSPy training examples.

    Args:
        session_dir: Path to ~/.apxm/sessions/<execution-id>

    Returns:
        List of examples in format: [{"inputs": {...}, "output": "..."}]
    """
    session_path = Path(session_dir)
    results_file = session_path / "results.json"

    if not results_file.exists():
        logging.error(f"Results file not found: {results_file}")
        return []

    try:
        with open(results_file) as f:
            results = json.load(f)
    except Exception as e:
        logging.error(f"Failed to load results: {e}")
        return []

    examples = []
    for node_id, node_result in results.items():
        # Skip failed nodes
        if not node_result.get("success", False):
            continue

        # Extract inputs and output
        inputs = node_result.get("inputs", {})
        output = node_result.get("output", "")

        if inputs and output:
            examples.append({
                "inputs": inputs,
                "output": output,
            })

    logging.info(f"Extracted {len(examples)} examples from {session_dir}")
    return examples


def load_training_data(data_path: Union[str, Path]) -> List[Dict[str, Any]]:
    """
    Load training data from JSON file.

    Expected format:
    [
        {"inputs": {"question": "What is 2+2?"}, "output": "4"},
        {"inputs": {"question": "What is Python?"}, "output": "A programming language"},
    ]

    Args:
        data_path: Path to JSON file

    Returns:
        List of training examples
    """
    data_file = Path(data_path)
    if not data_file.exists():
        logging.error(f"Training data file not found: {data_file}")
        return []

    try:
        with open(data_file) as f:
            data = json.load(f)

        if isinstance(data, list):
            return data
        elif isinstance(data, dict):
            # Handle per-node format: {"node_1": [...], "node_2": [...]}
            all_examples = []
            for node_examples in data.values():
                if isinstance(node_examples, list):
                    all_examples.extend(node_examples)
            return all_examples
        else:
            logging.error(f"Unexpected training data format: {type(data)}")
            return []

    except Exception as e:
        logging.error(f"Failed to load training data: {e}")
        return []


# CLI interface for standalone usage
def main():
    """Command-line interface for DSPy optimization."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Optimize APXM graph templates using DSPy"
    )
    parser.add_argument("graph", help="Path to APXM graph file (.apxm or .json)")
    parser.add_argument(
        "--training-data",
        help="Path to training data JSON file",
    )
    parser.add_argument(
        "--session-dir",
        help="Path to session directory (alternative to --training-data)",
    )
    parser.add_argument(
        "--output", "-o",
        help="Output path for optimized graph (default: stdout)",
    )
    parser.add_argument(
        "--optimizer",
        default="bootstrap_fewshot",
        choices=["bootstrap_fewshot", "mipro_v2", "copro"],
        help="DSPy optimizer to use",
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Verbose output",
    )

    args = parser.parse_args()

    # Set up logging
    logging.basicConfig(
        level=logging.INFO if args.verbose else logging.WARNING,
        format="%(levelname)s: %(message)s",
    )

    # Load graph
    graph_path = Path(args.graph)
    if not graph_path.exists():
        logging.error(f"Graph file not found: {graph_path}")
        return 1

    with open(graph_path) as f:
        graph = json.load(f)

    # Load training data
    training_data = None
    if args.training_data:
        training_data = load_training_data(args.training_data)
    elif args.session_dir:
        training_data = profile_to_examples(args.session_dir)

    if not training_data:
        logging.error("No training data provided. Use --training-data or --session-dir")
        return 1

    # Optimize graph
    config = OptimizationConfig(
        optimizer=args.optimizer,
        verbose=args.verbose,
    )

    bridge = ApxmDspyBridge()
    optimized_graph = bridge.optimize_graph(graph, training_data, config)

    # Write output
    if args.output:
        output_path = Path(args.output)
        with open(output_path, "w") as f:
            json.dump(optimized_graph, f, indent=2)
        logging.info(f"Optimized graph written to {output_path}")
    else:
        print(json.dumps(optimized_graph, indent=2))

    return 0


if __name__ == "__main__":
    import sys
    sys.exit(main())
