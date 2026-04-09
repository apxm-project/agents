"""Built-in metric functions for DSPy optimization."""

from __future__ import annotations

from collections import Counter

from .types import Metric


def token_overlap(gold, pred, trace=None, output_field: str = "output") -> float:
    """F1 token overlap between expected and actual output."""
    expected = getattr(gold, output_field, "").strip().lower().split()
    actual = getattr(pred, output_field, "").strip().lower().split()

    if not expected or not actual:
        return 0.0

    expected_counts = Counter(expected)
    actual_counts = Counter(actual)

    overlap = sum((expected_counts & actual_counts).values())
    precision = overlap / len(actual) if actual else 0.0
    recall = overlap / len(expected) if expected else 0.0

    if precision + recall == 0:
        return 0.0
    return 2 * precision * recall / (precision + recall)


def exact_match(gold, pred, trace=None, output_field: str = "output") -> bool:
    """Exact string match between expected and actual output."""
    expected = getattr(gold, output_field, "").strip()
    actual = getattr(pred, output_field, "").strip()
    return expected == actual


def contains_match(gold, pred, trace=None, output_field: str = "output") -> bool:
    """Check if expected output is a substring of actual output."""
    expected = getattr(gold, output_field, "").strip()
    actual = getattr(pred, output_field, "").strip()
    return expected in actual


def build_metric(metric_name: str, output_field: str = "output"):
    """Build a metric function by name.

    Supported metrics: token_overlap, exact_match, contains, llm_judge
    """
    dispatch = {
        Metric.EXACT_MATCH: exact_match,
        Metric.CONTAINS: contains_match,
        Metric.TOKEN_OVERLAP: token_overlap,
    }

    base_fn = dispatch.get(metric_name)
    if base_fn is not None:
        def metric(gold, pred, trace=None):
            return base_fn(gold, pred, trace, output_field)
        return metric

    if metric_name == Metric.LLM_JUDGE:
        import dspy
        judge = dspy.Predict("expected, actual -> score: bool")
        def metric(gold, pred, trace=None):
            expected = getattr(gold, output_field, "")
            actual = getattr(pred, output_field, "")
            result = judge(expected=expected, actual=actual)
            return result.score
        return metric

    # Default: token_overlap
    return build_metric(Metric.TOKEN_OVERLAP, output_field)
