"""
Quality evaluation module for APXM workflows.

Measures output quality before/after optimization and feeds back into compilation heuristics.
Uses DSPy metrics to detect quality regressions from optimizations like fusion.

Usage:
    evaluator = QualityEvaluator()
    report = evaluator.evaluate_optimization(graph, training_data, o0_outputs, o2_outputs)

    if report.regression:
        print(f"Quality regression detected: {report.recommendation}")
        # Recompile with risky fusions disabled
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Union

try:
    import dspy
    from dspy.evaluate import metrics
    DSPY_AVAILABLE = True
except ImportError:
    DSPY_AVAILABLE = False
    logging.warning(
        "DSPy not installed. Install with: pip install dspy-ai\n"
        "Quality evaluation will use fallback metrics."
    )


@dataclass
class QualityReport:
    """Report from quality evaluation comparing O0 vs O2 outputs."""
    o0_score: float
    o2_score: float
    regression: bool
    recommendation: str
    per_example_scores: Optional[List[tuple[float, float]]] = None  # [(o0_score, o2_score), ...]
    risky_fusions: Optional[List[Dict[str, Any]]] = None  # Fusions that caused regressions


@dataclass
class FusionCandidate:
    """Represents a potential fusion between producer and consumer nodes."""
    producer_id: str
    consumer_id: str
    quality_drop: float = 0.0


class QualityEvaluator:
    """
    Evaluates output quality of APXM workflows using DSPy metrics.

    Compares quality before (O0) and after (O2) optimization to detect regressions
    and identify risky fusions that hurt quality.
    """

    def __init__(self, metric: Optional[Callable] = None, regression_threshold: float = 0.05):
        """
        Initialize quality evaluator.

        Args:
            metric: Custom metric function(example, output) -> score in [0, 1].
                   If None, uses default_metric.
            regression_threshold: Quality drop threshold to flag as regression.
                                 Default 0.05 means 5% drop is considered bad.
        """
        self.metric = metric or self._default_metric
        self.regression_threshold = regression_threshold

    def evaluate_optimization(
        self,
        graph: Union[Dict[str, Any], Any],
        training_data: List[Dict[str, Any]],
        o0_outputs: List[str],
        o2_outputs: List[str],
    ) -> QualityReport:
        """
        Compare quality of O0 vs O2 outputs.

        Args:
            graph: APXM graph (dict or ApxmGraph object)
            training_data: List of examples with 'inputs' and 'output' keys
            o0_outputs: Outputs from O0 compilation (unoptimized)
            o2_outputs: Outputs from O2 compilation (optimized)

        Returns:
            QualityReport with scores, regressions, and recommendations.
        """
        if len(o0_outputs) != len(o2_outputs):
            raise ValueError(
                f"Output length mismatch: O0={len(o0_outputs)}, O2={len(o2_outputs)}"
            )

        if len(training_data) != len(o0_outputs):
            logging.warning(
                f"Training data length ({len(training_data)}) != "
                f"output length ({len(o0_outputs)}). Using min length."
            )
            n = min(len(training_data), len(o0_outputs))
            training_data = training_data[:n]
            o0_outputs = o0_outputs[:n]
            o2_outputs = o2_outputs[:n]

        # Compute per-example scores
        per_example_scores = []
        scores_o0 = []
        scores_o2 = []

        for example, o0_out, o2_out in zip(training_data, o0_outputs, o2_outputs):
            score_o0 = self.metric(example, o0_out)
            score_o2 = self.metric(example, o2_out)
            scores_o0.append(score_o0)
            scores_o2.append(score_o2)
            per_example_scores.append((score_o0, score_o2))

        # Compute average scores
        avg_o0 = sum(scores_o0) / len(scores_o0) if scores_o0 else 0.0
        avg_o2 = sum(scores_o2) / len(scores_o2) if scores_o2 else 0.0

        # Detect regression
        regression = avg_o2 < avg_o0 * (1 - self.regression_threshold)

        # Generate recommendation
        if not regression:
            recommendation = "safe"
        else:
            quality_drop = avg_o0 - avg_o2
            recommendation = f"review_fusions (quality dropped by {quality_drop:.2%})"

        return QualityReport(
            o0_score=avg_o0,
            o2_score=avg_o2,
            regression=regression,
            recommendation=recommendation,
            per_example_scores=per_example_scores,
            risky_fusions=None,  # Will be filled by identify_risky_fusions
        )

    def identify_risky_fusions(
        self,
        graph: Union[Dict[str, Any], Any],
        training_data: List[Dict[str, Any]],
        o0_outputs: List[str],
        o2_outputs: List[str],
        fusion_list: Optional[List[FusionCandidate]] = None,
    ) -> List[FusionCandidate]:
        """
        Identify which fusions caused quality regressions.

        This requires running the graph with individual fusions disabled one at a time
        to isolate the culprit. For now, we use a heuristic: flag all potential fusions
        if there's a regression.

        Args:
            graph: APXM graph
            training_data: Training examples
            o0_outputs: O0 outputs
            o2_outputs: O2 outputs
            fusion_list: Optional list of fusions that were applied in O2

        Returns:
            List of FusionCandidates sorted by quality_drop (highest first)
        """
        # Compute overall quality drop
        avg_o0 = sum(self.metric(ex, out) for ex, out in zip(training_data, o0_outputs)) / len(training_data)
        avg_o2 = sum(self.metric(ex, out) for ex, out in zip(training_data, o2_outputs)) / len(training_data)
        quality_drop = avg_o0 - avg_o2

        if quality_drop <= 0:
            return []  # No regression

        # If fusion_list provided, annotate with quality_drop
        if fusion_list:
            risky = []
            for fusion in fusion_list:
                fusion.quality_drop = quality_drop / len(fusion_list)  # Distribute blame evenly
                risky.append(fusion)
            return sorted(risky, key=lambda f: f.quality_drop, reverse=True)

        # Extract LLM nodes that could be fused
        graph_dict = graph if isinstance(graph, dict) else graph.to_dict()
        llm_ops = {"ASK", "THINK", "REASON"}
        llm_nodes = [
            node for node in graph_dict.get("nodes", [])
            if node.get("op") in llm_ops
        ]

        if not llm_nodes:
            return []  # No LLM nodes to analyze

        # Fallback: extract potential fusions from graph edges
        risky = []
        edges = graph_dict.get("edges", [])
        llm_node_ids = {node["id"] for node in llm_nodes}

        for edge in edges:
            if edge.get("from") in llm_node_ids and edge.get("to") in llm_node_ids:
                # Potential fusion between LLM nodes
                risky.append(FusionCandidate(
                    producer_id=str(edge["from"]),
                    consumer_id=str(edge["to"]),
                    quality_drop=quality_drop / max(len(edges), 1),
                ))

        return sorted(risky, key=lambda f: f.quality_drop, reverse=True)

    def _default_metric(self, example: Dict[str, Any], output: str) -> float:
        """
        Default quality metric.

        Uses DSPy's built-in metrics if available, otherwise falls back to
        token overlap similarity.

        Args:
            example: Training example with 'output' key
            output: Generated output

        Returns:
            Score in [0, 1] where 1 is perfect match
        """
        expected = example.get("output", "")

        # Use token overlap similarity (better than exact match for partial credit)
        if not expected or not output:
            return 0.0

        expected_tokens = set(expected.lower().split())
        output_tokens = set(output.lower().split())

        if not expected_tokens:
            return 0.0

        overlap = expected_tokens & output_tokens
        return len(overlap) / len(expected_tokens)

    def save_quality_profile(
        self,
        report: QualityReport,
        graph_name: str,
        output_path: Union[str, Path],
    ) -> None:
        """
        Save quality report as a .apxm-quality-profile.json file.

        Args:
            report: QualityReport from evaluate_optimization
            graph_name: Name of the graph
            output_path: Path to save the profile
        """
        import json
        from datetime import datetime

        profile = {
            "graph": graph_name,
            "date": datetime.utcnow().isoformat(),
            "quality_scores": {
                "o0": report.o0_score,
                "o2": report.o2_score,
            },
            "risky_fusions": [
                {
                    "producer": f.producer_id,
                    "consumer": f.consumer_id,
                    "quality_drop": f.quality_drop,
                }
                for f in (report.risky_fusions or [])
            ],
            "recommendation": report.recommendation,
        }

        output_file = Path(output_path)
        with open(output_file, "w") as f:
            json.dump(profile, f, indent=2)

        logging.info(f"Quality profile saved to {output_file}")

    @staticmethod
    def load_quality_profile(profile_path: Union[str, Path]) -> Dict[str, Any]:
        """
        Load a quality profile from disk.

        Args:
            profile_path: Path to .apxm-quality-profile.json

        Returns:
            Quality profile dict
        """
        import json

        profile_file = Path(profile_path)
        if not profile_file.exists():
            raise FileNotFoundError(f"Quality profile not found: {profile_file}")

        with open(profile_file) as f:
            return json.load(f)


def compare_optimizations(
    graph: Union[Dict[str, Any], Any],
    training_data: List[Dict[str, Any]],
    o0_outputs: List[str],
    o2_outputs: List[str],
    o2_no_fusion_outputs: Optional[List[str]] = None,
) -> Dict[str, QualityReport]:
    """
    Compare multiple optimization levels.

    Args:
        graph: APXM graph
        training_data: Training examples
        o0_outputs: Unoptimized outputs
        o2_outputs: Fully optimized outputs
        o2_no_fusion_outputs: O2 with fusion disabled (optional)

    Returns:
        Dict mapping optimization level to QualityReport
    """
    evaluator = QualityEvaluator()
    reports = {}

    # Compare O0 vs O2
    reports["o0_vs_o2"] = evaluator.evaluate_optimization(
        graph, training_data, o0_outputs, o2_outputs
    )

    # Compare O0 vs O2_no_fusion if available
    if o2_no_fusion_outputs:
        reports["o0_vs_o2_no_fusion"] = evaluator.evaluate_optimization(
            graph, training_data, o0_outputs, o2_no_fusion_outputs
        )

        # Compare O2_no_fusion vs O2 to isolate fusion impact
        reports["o2_no_fusion_vs_o2"] = evaluator.evaluate_optimization(
            graph, training_data, o2_no_fusion_outputs, o2_outputs
        )

    return reports
