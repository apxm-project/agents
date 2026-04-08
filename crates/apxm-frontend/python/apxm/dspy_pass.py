#!/usr/bin/env python3
"""
DSPy Compiler Pass for APXM

Optimizes LLM operation templates at compile time using DSPy.
Called by Rust compiler via subprocess or standalone for testing.

Usage:
    # Standalone
    python3 -m apxm.dspy_pass --template "{{input}}" --training-data examples.json

    # As compiler subprocess (called by Rust)
    python3 -m apxm.dspy_pass optimize-template \
        --template "Answer: {{question}}" \
        --training-data /path/to/examples.json \
        --optimizer bootstrap_fewshot \
        --target balanced
"""

import argparse
import json
import sys
from pathlib import Path
from typing import List, Dict, Any, Optional
from dataclasses import dataclass, asdict

try:
    import dspy
    from dspy import Example
    from dspy.teleprompt import LabeledFewShot, BootstrapFewShot
    DSPY_AVAILABLE = True
except ImportError:
    DSPY_AVAILABLE = False
    print("ERROR: DSPy not installed. Run: pip install dspy-ai", file=sys.stderr)
    sys.exit(1)


@dataclass
class OptimizedTemplate:
    """Result of DSPy template optimization."""
    template: str
    version: str
    score: float
    metadata: Dict[str, Any]


class DspyCompilerPass:
    """
    DSPy optimization as an APXM compiler pass.

    This class implements prompt optimization using DSPy algorithms (LabeledFewShot,
    BootstrapFewShot, MIPROv2) and per-target metrics (quality, latency, tokens, balanced).

    The pass operates on individual template strings extracted from MLIR operations,
    optimizes them using training data, and returns enhanced templates with few-shot
    examples and improved phrasing.
    """

    def __init__(self, lm_config: Optional[Dict[str, Any]] = None):
        """
        Initialize DSPy compiler pass.

        Args:
            lm_config: Language model configuration with keys:
                - model: Model name (default: "gpt-4o-mini")
                - max_tokens: Max output tokens (default: 2048)
                - api_key: API key (optional, uses env var)
                - api_base: Custom API endpoint (optional)
        """
        if lm_config is None:
            lm_config = {"model": "gpt-4o-mini", "max_tokens": 2048}

        self.lm_config = lm_config
        self.lm = None

        # Try to configure LM (may fail if no API key, that's OK for labeled_fewshot)
        try:
            self.lm = dspy.LM(
                model=lm_config.get("model", "gpt-4o-mini"),
                max_tokens=lm_config.get("max_tokens", 2048),
            )
            dspy.configure(lm=self.lm)
        except Exception as e:
            print(f"WARNING: Failed to configure DSPy LM: {e}", file=sys.stderr)
            print("         LabeledFewShot will work, but BootstrapFewShot requires API key", file=sys.stderr)

    def optimize_template(
        self,
        template: str,
        training_data: List[Example],
        optimizer: str = "bootstrap_fewshot",
        target: str = "balanced",
    ) -> OptimizedTemplate:
        """
        Optimize a single template using DSPy.

        Args:
            template: Original template string with {{placeholders}}
            training_data: List of DSPy Examples
            optimizer: "labeled_fewshot", "bootstrap_fewshot", or "mipro_v2"
            target: "quality", "latency", "tokens", or "balanced"

        Returns:
            OptimizedTemplate with optimized prompt, version, score, metadata
        """
        if not training_data:
            raise ValueError("Training data is required for DSPy optimization")

        # 1. Convert template to DSPy Signature
        signature = self._template_to_signature(template)
        print(f"  Signature: {signature}", file=sys.stderr)

        # 2. Create predictor
        predictor = dspy.Predict(signature)

        # 3. Select metric based on target
        metric = self._get_metric_for_target(target)

        # 4. Run optimizer
        print(f"  Running {optimizer} with {len(training_data)} examples...", file=sys.stderr)

        if optimizer == "labeled_fewshot":
            teleprompter = LabeledFewShot(k=3)
            optimized = teleprompter.compile(predictor, trainset=training_data)

        elif optimizer == "bootstrap_fewshot":
            if self.lm is None:
                print("WARNING: BootstrapFewShot requires LM, falling back to LabeledFewShot", file=sys.stderr)
                teleprompter = LabeledFewShot(k=3)
            else:
                teleprompter = BootstrapFewShot(
                    max_bootstrapped_demos=5,
                    max_labeled_demos=10,
                    metric=metric,
                )
            optimized = teleprompter.compile(predictor, trainset=training_data)

        elif optimizer == "mipro_v2":
            # MIPROv2 not yet implemented
            raise NotImplementedError("MIPROv2 optimizer not yet available in APXM v1.0")

        else:
            raise ValueError(f"Unknown optimizer: {optimizer}")

        # 5. Extract optimized template
        optimized_template_str = self._extract_template(optimized, signature)
        print(f"  Optimized template: {len(optimized_template_str)} chars", file=sys.stderr)

        # 6. Evaluate quality (simple metric for now)
        score = self._evaluate(optimized, training_data[:5], metric)  # Evaluate on first 5
        print(f"  Quality score: {score:.3f}", file=sys.stderr)

        return OptimizedTemplate(
            template=optimized_template_str,
            version=dspy.__version__,
            score=score,
            metadata={
                "optimizer": optimizer,
                "target": target,
                "num_examples": len(training_data),
                "signature": signature,
                "original_template": template,
            }
        )

    def _template_to_signature(self, template: str) -> str:
        """
        Convert APXM template to DSPy signature.

        Examples:
            "{{question}}" → "question -> output"
            "Context: {{context}}\nQ: {{question}}" → "context, question -> output"
            "{{input}}" → "input -> output"
        """
        import re
        placeholders = re.findall(r'\{\{(\w+)\}\}', template)

        if not placeholders:
            # No placeholders, assume single input
            return "input -> output"

        # Deduplicate and sort
        inputs = sorted(set(placeholders))

        # Create signature: "question, context -> answer"
        inputs_str = ", ".join(inputs)
        return f"{inputs_str} -> output"

    def _extract_template(self, optimized_predictor, signature: str) -> str:
        """
        Extract optimized template from DSPy predictor.

        DSPy stores few-shot demonstrations in predictor.demos.
        We reconstruct the template with examples injected.
        """
        demos = getattr(optimized_predictor, 'demos', [])

        if not demos:
            # No demos added, return placeholder-based template
            input_fields = signature.split(" -> ")[0].split(", ")
            placeholders = " ".join([f"{{{{{f}}}}}" for f in input_fields])
            return placeholders

        # Build few-shot template with examples
        template_parts = ["Examples:"]

        for i, demo in enumerate(demos[:5]):  # Limit to 5 examples to avoid token bloat
            template_parts.append(f"Example {i+1}:")

            # Input fields
            for key, value in demo.items():
                if key != 'output':
                    # Truncate long values to 200 chars
                    value_str = str(value)
                    if len(value_str) > 200:
                        value_str = value_str[:197] + "..."
                    template_parts.append(f"  {key}: {value_str}")

            # Output
            output = demo.get('output', '')
            if len(str(output)) > 200:
                output = str(output)[:197] + "..."
            template_parts.append(f"  output: {output}")
            template_parts.append("")

        # Add final prompt with placeholders
        template_parts.append("Now complete the following:")

        # Extract input field names from signature
        input_fields = signature.split(" -> ")[0].split(", ")
        for field in input_fields:
            template_parts.append(f"{field}: {{{{{field}}}}}")

        template_parts.append("output:")

        return "\n".join(template_parts)

    def _get_metric_for_target(self, target: str):
        """
        Get DSPy metric function for optimization target.

        Metrics return a score in [0, 1] where higher is better.
        """
        if target == "quality":
            return self._quality_metric
        elif target == "latency":
            return self._latency_metric
        elif target == "tokens":
            return self._tokens_metric
        else:  # balanced
            return self._balanced_metric

    def _quality_metric(self, example, pred, trace=None):
        """Metric optimizing for pure accuracy."""
        # Simple exact match (could use semantic similarity with embeddings)
        return 1.0 if str(example.output) == str(pred.output) else 0.0

    def _latency_metric(self, example, pred, trace=None):
        """
        Metric optimizing for latency (shorter outputs = faster inference).

        Trade-off: 70% quality, 30% brevity
        """
        quality = 1.0 if str(example.output) == str(pred.output) else 0.0

        # Reward brevity (fewer tokens)
        output_tokens = len(str(pred.output).split())
        brevity = 1.0 / max(1.0, output_tokens / 100.0)  # Normalize to 100 tokens
        brevity = min(1.0, brevity)  # Cap at 1.0

        return quality * 0.7 + brevity * 0.3

    def _tokens_metric(self, example, pred, trace=None):
        """
        Metric optimizing for token efficiency.

        Trade-off: 60% quality, 40% token minimization
        """
        quality = 1.0 if str(example.output) == str(pred.output) else 0.0

        # Token efficiency (penalize long outputs)
        output_tokens = len(str(pred.output).split())
        efficiency = 1.0 - min(1.0, output_tokens / 2000.0)  # Normalize to 2k tokens

        return quality * 0.6 + efficiency * 0.4

    def _balanced_metric(self, example, pred, trace=None):
        """
        Balanced multi-objective metric.

        Trade-off: 70% quality, 30% token efficiency
        """
        quality = 1.0 if str(example.output) == str(pred.output) else 0.0

        # Token efficiency
        output_tokens = len(str(pred.output).split())
        token_score = 1.0 - min(1.0, output_tokens / 2000.0)

        return quality * 0.7 + token_score * 0.3

    def _evaluate(self, predictor, test_data: List[Example], metric) -> float:
        """
        Evaluate optimized predictor on test data.

        Returns average metric score across test examples.
        """
        if not test_data:
            return 0.0

        total_score = 0.0
        num_evaluated = 0

        for example in test_data:
            try:
                # Extract input fields (everything except 'output')
                inputs = {k: v for k, v in example.items() if k != 'output'}

                # Run predictor
                pred = predictor(**inputs)

                # Compute metric
                score = metric(example, pred)
                total_score += score
                num_evaluated += 1

            except Exception as e:
                print(f"WARNING: Evaluation failed on example: {e}", file=sys.stderr)
                continue

        if num_evaluated == 0:
            return 0.0

        return total_score / num_evaluated


def load_training_data(path: Path) -> List[Example]:
    """
    Load training data from JSON file.

    Format:
        [
          {"inputs": {"question": "..."}, "output": "..."},
          ...
        ]

    Returns:
        List of DSPy Examples
    """
    with open(path) as f:
        data = json.load(f)

    examples = []
    for item in data:
        if "inputs" not in item or "output" not in item:
            print(f"WARNING: Skipping malformed example: {item}", file=sys.stderr)
            continue

        # Create DSPy Example
        ex = Example(**item["inputs"], output=item["output"])
        examples.append(ex.with_inputs(**item["inputs"]))

    return examples


def main():
    """CLI entry point for DSPy compiler pass."""
    parser = argparse.ArgumentParser(
        description="DSPy compiler pass for APXM",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Optimize a template with training data
  python3 -m apxm.dspy_pass --template "{{input}}" --training-data examples.json

  # Use specific optimizer and target
  python3 -m apxm.dspy_pass --template "Answer: {{question}}" \\
      --training-data examples.json \\
      --optimizer bootstrap_fewshot \\
      --target latency

  # Save result to file
  python3 -m apxm.dspy_pass --template "{{input}}" \\
      --training-data examples.json \\
      --output optimized.json
        """
    )

    parser.add_argument(
        "--template",
        required=True,
        help="Template string to optimize (e.g., 'Answer: {{question}}')"
    )
    parser.add_argument(
        "--training-data",
        required=True,
        help="Path to training data JSON file"
    )
    parser.add_argument(
        "--optimizer",
        default="labeled_fewshot",
        choices=["labeled_fewshot", "bootstrap_fewshot", "mipro_v2"],
        help="DSPy optimizer to use (default: labeled_fewshot)"
    )
    parser.add_argument(
        "--target",
        default="balanced",
        choices=["quality", "latency", "tokens", "balanced"],
        help="Optimization target (default: balanced)"
    )
    parser.add_argument(
        "--output",
        help="Output path for result JSON (default: stdout)"
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Enable verbose logging"
    )

    args = parser.parse_args()

    # Load training data
    print(f"Loading training data from {args.training_data}...", file=sys.stderr)
    training_data = load_training_data(Path(args.training_data))
    print(f"Loaded {len(training_data)} examples", file=sys.stderr)

    # Run optimization
    print(f"Optimizing template with {args.optimizer}...", file=sys.stderr)
    compiler_pass = DspyCompilerPass()

    result = compiler_pass.optimize_template(
        template=args.template,
        training_data=training_data,
        optimizer=args.optimizer,
        target=args.target,
    )

    # Output result as JSON
    output_data = asdict(result)

    if args.output:
        with open(args.output, 'w') as f:
            json.dump(output_data, f, indent=2)
        print(f"\nResult saved to {args.output}", file=sys.stderr)
    else:
        print(json.dumps(output_data, indent=2))

    print("\n=== Optimization Complete ===", file=sys.stderr)
    print(f"Score: {result.score:.3f}", file=sys.stderr)
    print(f"Template length: {len(result.template)} chars", file=sys.stderr)


if __name__ == "__main__":
    main()
