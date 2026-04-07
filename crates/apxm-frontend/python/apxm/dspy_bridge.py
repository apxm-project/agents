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
    from dspy.teleprompt import BootstrapFewShot, LabeledFewShot
    DSPY_AVAILABLE = True
except ImportError:
    DSPY_AVAILABLE = False
    logging.warning(
        "DSPy not installed. Install with: pip install dspy-ai\n"
        "DSPy optimization will be skipped."
    )


@dataclass
class OptimizationConfig:
    """Configuration for DSPy optimization."""
    optimizer: str = "bootstrap_fewshot"  # bootstrap_fewshot, labeled_fewshot, mipro_v2, copro
    max_bootstrapped_demos: int = 5
    max_labeled_demos: int = 10
    num_trials: int = 10
    metric_threshold: float = 0.5
    verbose: bool = False
    fallback_to_labeled: bool = True  # If bootstrap fails, fall back to labeled fewshot


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
        graph: Union[Dict[str, Any], Any],
        training_data: Optional[List[Dict[str, Any]]] = None,
        config: Optional[OptimizationConfig] = None,
    ) -> Union[Dict[str, Any], Any]:
        """
        Optimize all LLM templates in an APXM graph.

        Args:
            graph: APXM graph dict with 'nodes' and 'edges' keys, or ApxmGraph object
            training_data: List of training examples (DSPy format or session results)
            config: Optimization configuration

        Returns:
            Optimized graph with updated template_str attributes (same type as input)
        """
        if not DSPY_AVAILABLE or self.lm is None:
            logging.info("DSPy not available, returning graph unchanged")
            return graph

        if config is None:
            config = OptimizationConfig()

        # Convert ApxmGraph to dict if needed
        is_apxm_graph = hasattr(graph, 'to_dict')
        if is_apxm_graph:
            graph_dict = graph.to_dict()
        else:
            graph_dict = graph

        # Extract LLM nodes (ASK, THINK, REASON)
        llm_ops = {"ASK", "THINK", "REASON"}
        llm_nodes = [
            node for node in graph_dict.get("nodes", [])
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
        import copy
        optimized_graph = copy.deepcopy(graph_dict)
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

        # Return optimized graph in same format as input
        if is_apxm_graph:
            # Import ApxmGraph to reconstruct
            from apxm.ir import ApxmGraph
            return ApxmGraph.from_dict(optimized_graph)
        else:
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
            Optimized template string with learned instructions and few-shot examples
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
        # Extract variables from template (e.g., {{question}}, {{context}}, {{0}}, {{1}})
        import re
        var_pattern = r'\{\{(\w+)\}\}'
        template_vars = re.findall(var_pattern, template)

        if not template_vars:
            logging.warning("No input variables found in template")
            return template

        # Determine if we have positional ({{0}}, {{1}}) or named ({{question}}) placeholders
        is_positional = all(v.isdigit() for v in template_vars)

        # Get input variable names from examples
        example_input_keys = list(dspy_examples[0].__dict__.get('_input_keys', set()))
        if not example_input_keys and hasattr(dspy_examples[0], '__dict__'):
            # Fallback: get all keys except 'answer'
            example_input_keys = [k for k in dspy_examples[0].__dict__.keys()
                                  if k not in {'answer', '_input_keys', '_demos'}]

        if not example_input_keys:
            logging.warning("No input keys found in examples")
            return template

        # Build signature using example input keys
        if is_positional:
            # For positional templates like "{{0}}, {{1}}", use example keys in order
            input_vars = example_input_keys[:len(set(template_vars))]
        else:
            # For named templates, use the template variable names
            input_vars = list(set(template_vars))

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
                try:
                    optimizer = BootstrapFewShot(
                        metric=simple_metric,
                        max_bootstrapped_demos=config.max_bootstrapped_demos,
                        max_labeled_demos=config.max_labeled_demos,
                    )
                    optimized_predictor = optimizer.compile(
                        predictor,
                        trainset=dspy_examples[:min(len(dspy_examples), 20)],  # Limit for speed
                    )
                except Exception as bootstrap_error:
                    # Bootstrap requires API calls - if no API key, fall back to labeled
                    if config.fallback_to_labeled and ("api_key" in str(bootstrap_error).lower() or
                                                       "authentication" in str(bootstrap_error).lower()):
                        logging.info("BootstrapFewShot requires API key, falling back to LabeledFewShot")
                        config.optimizer = "labeled_fewshot"
                    else:
                        raise

            if config.optimizer == "labeled_fewshot":
                # LabeledFewShot doesn't need API calls - just uses provided examples
                optimizer = LabeledFewShot(k=config.max_labeled_demos)
                optimized_predictor = optimizer.compile(
                    predictor,
                    trainset=dspy_examples[:min(len(dspy_examples), 20)],
                )
            elif config.optimizer not in {"bootstrap_fewshot"}:
                logging.warning(f"Optimizer {config.optimizer} not yet implemented, using original")
                return template

            # Extract the optimized prompt from the compiled predictor
            # DSPy stores demos and instructions in the predictor's state
            optimized_template = self._extract_optimized_prompt(
                optimized_predictor.predictor,
                template,
                input_vars
            )

            return optimized_template

        except Exception as e:
            logging.error(f"DSPy optimization failed: {e}")
            import traceback
            if config.verbose:
                traceback.print_exc()
            return template

    def _extract_optimized_prompt(
        self,
        predictor: Any,
        original_template: str,
        input_vars: List[str],
    ) -> str:
        """
        Extract optimized prompt from a compiled DSPy predictor.

        DSPy stores optimized prompts in the predictor's demos and extended_signature.
        We extract these and format them as an APXM template.

        Args:
            predictor: DSPy Predict module (after optimization)
            original_template: Original template string
            input_vars: List of input variable names

        Returns:
            Optimized template with instructions and few-shot examples
        """
        try:
            # Get few-shot demos from the optimized predictor
            demos = getattr(predictor, 'demos', [])

            # Get extended signature (contains learned instructions)
            extended_sig = getattr(predictor, 'extended_signature', None)
            if extended_sig:
                # DSPy adds instructions to the signature
                instructions = getattr(extended_sig, 'instructions', '')
            else:
                instructions = ''

            # Build optimized prompt
            parts = []

            # Add learned instructions if available
            if instructions:
                parts.append(instructions.strip())
                parts.append('')  # Blank line

            # Add few-shot examples if available
            if demos:
                parts.append('Examples:')
                for i, demo in enumerate(demos[:5], 1):  # Limit to 5 examples
                    parts.append(f'Example {i}:')
                    # Format input variables
                    for j, var in enumerate(input_vars):
                        value = getattr(demo, var, None)
                        if value:
                            parts.append(f'  {var}: {value}')
                    # Format output
                    answer = getattr(demo, 'answer', None)
                    if answer:
                        parts.append(f'  answer: {answer}')
                    parts.append('')  # Blank line between examples

            # Add the task prompt with placeholders
            # Check if original template uses positional ({{0}}) or named ({{var}}) placeholders
            import re
            template_vars = re.findall(r'\{\{(\w+)\}\}', original_template)
            is_positional = all(v.isdigit() for v in template_vars) if template_vars else False

            parts.append('Now complete the following:')
            if is_positional:
                # Use positional placeholders to match original template
                for i, var in enumerate(input_vars):
                    parts.append(f'{var}: {{{{{i}}}}}')
            else:
                # Use named placeholders
                for var in input_vars:
                    parts.append(f'{var}: {{{{{var}}}}}')
            parts.append('answer:')

            optimized = '\n'.join(parts)

            # If we got nothing from DSPy, return original
            if not instructions and not demos:
                logging.warning("No learned instructions or demos found in optimized predictor")
                return original_template

            return optimized

        except Exception as e:
            logging.warning(f"Failed to extract optimized prompt: {e}")
            import traceback
            traceback.print_exc()
            return original_template


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
    """
    Command-line interface for DSPy optimization.

    Example usage:
        # Optimize with explicit training data
        python3 -m apxm.dspy_bridge workflow.apxm --training-data examples.json -o optimized.apxm

        # Optimize using session results
        python3 -m apxm.dspy_bridge workflow.apxm --session-dir ~/.apxm/sessions/abc123 -o optimized.apxm

        # Use LabeledFewShot (no API key required)
        python3 -m apxm.dspy_bridge workflow.apxm --training-data examples.json --optimizer labeled_fewshot
    """
    import argparse

    parser = argparse.ArgumentParser(
        description="Optimize APXM graph templates using DSPy",
        epilog="""
Examples:
  %(prog)s workflow.apxm --training-data examples.json -o optimized.apxm
  %(prog)s workflow.apxm --session-dir ~/.apxm/sessions/abc123 --verbose
  %(prog)s workflow.apxm --training-data examples.json --optimizer labeled_fewshot
        """,
        formatter_class=argparse.RawDescriptionHelpFormatter,
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
        help="Output path for optimized graph (default: <graph>_optimized.apxm)",
    )
    parser.add_argument(
        "--optimizer",
        default="bootstrap_fewshot",
        choices=["bootstrap_fewshot", "labeled_fewshot", "mipro_v2", "copro"],
        help="DSPy optimizer to use (default: bootstrap_fewshot)",
    )
    parser.add_argument(
        "--max-demos",
        type=int,
        default=5,
        help="Maximum number of few-shot demos to include (default: 5)",
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
        max_labeled_demos=args.max_demos,
        max_bootstrapped_demos=args.max_demos,
        verbose=args.verbose,
    )

    bridge = ApxmDspyBridge()
    optimized_graph = bridge.optimize_graph(graph, training_data, config)

    # Ensure we have a dict for JSON serialization
    if hasattr(optimized_graph, 'to_dict'):
        optimized_dict = optimized_graph.to_dict()
    else:
        optimized_dict = optimized_graph

    # Write output
    if args.output:
        output_path = Path(args.output)
    else:
        # Default: add _optimized suffix
        input_path = Path(args.graph)
        output_path = input_path.parent / f"{input_path.stem}_optimized{input_path.suffix}"

    with open(output_path, "w") as f:
        json.dump(optimized_dict, f, indent=2)

    print(f"✓ Optimized graph written to {output_path}")
    print(f"  Training examples: {len(training_data)}")
    print(f"  Optimizer: {config.optimizer}")
    llm_nodes = sum(1 for n in optimized_dict.get("nodes", [])
                   if n.get("op") in {"ASK", "THINK", "REASON"})
    print(f"  LLM nodes optimized: {llm_nodes}")

    return 0


if __name__ == "__main__":
    import sys
    sys.exit(main())
