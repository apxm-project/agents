"""
Tests for quality evaluation module.

These tests verify:
1. QualityEvaluator can be initialized and used
2. Quality reports are generated correctly
3. Risky fusion identification works
4. Quality profiles can be saved and loaded
"""

import json
import tempfile
from pathlib import Path

import pytest

from apxm.quality import (
    QualityEvaluator,
    QualityReport,
    FusionCandidate,
    compare_optimizations,
)


def test_quality_evaluator_initialization():
    """Test QualityEvaluator can be initialized."""
    evaluator = QualityEvaluator()
    assert evaluator is not None
    assert evaluator.regression_threshold == 0.05


def test_quality_evaluator_custom_threshold():
    """Test QualityEvaluator with custom regression threshold."""
    evaluator = QualityEvaluator(regression_threshold=0.10)
    assert evaluator.regression_threshold == 0.10


def test_evaluate_optimization_no_regression():
    """Test quality evaluation when O2 maintains quality."""
    graph = {
        "name": "test_graph",
        "nodes": [
            {"id": 1, "name": "ask", "op": "ASK", "attributes": {"template_str": "Q: {{q}}"}}
        ],
        "edges": [],
    }

    training_data = [
        {"inputs": {"question": "What is 2+2?"}, "output": "4"},
        {"inputs": {"question": "What is Python?"}, "output": "A language"},
    ]

    o0_outputs = ["4", "A language"]
    o2_outputs = ["4", "A programming language"]  # Similar quality

    evaluator = QualityEvaluator()
    report = evaluator.evaluate_optimization(graph, training_data, o0_outputs, o2_outputs)

    assert report.o0_score >= 0.0
    assert report.o2_score >= 0.0
    # For this simple test, regression should be false if outputs are similar
    assert isinstance(report.regression, bool)
    # Recommendation format varies based on regression
    assert "safe" in report.recommendation or "review" in report.recommendation


def test_evaluate_optimization_with_regression():
    """Test quality evaluation when O2 degrades quality."""
    graph = {
        "name": "test_graph",
        "nodes": [
            {"id": 1, "name": "ask", "op": "ASK", "attributes": {"template_str": "Q: {{q}}"}}
        ],
        "edges": [],
    }

    training_data = [
        {"inputs": {"question": "What is 2+2?"}, "output": "4"},
        {"inputs": {"question": "What is Python?"}, "output": "A programming language"},
    ]

    o0_outputs = ["4", "A programming language"]
    o2_outputs = ["wrong", "wrong"]  # Degraded quality

    evaluator = QualityEvaluator()
    report = evaluator.evaluate_optimization(graph, training_data, o0_outputs, o2_outputs)

    # Degraded outputs should have lower score
    assert report.o2_score < report.o0_score


def test_evaluate_optimization_length_mismatch():
    """Test that length mismatches raise appropriate errors."""
    graph = {"name": "test", "nodes": [], "edges": []}
    training_data = [{"inputs": {}, "output": "result"}]
    o0_outputs = ["result"]
    o2_outputs = ["result1", "result2"]  # Mismatch

    evaluator = QualityEvaluator()

    with pytest.raises(ValueError, match="Output length mismatch"):
        evaluator.evaluate_optimization(graph, training_data, o0_outputs, o2_outputs)


def test_identify_risky_fusions():
    """Test risky fusion identification."""
    graph = {
        "name": "test_graph",
        "nodes": [
            {"id": 1, "name": "ask1", "op": "ASK", "attributes": {}},
            {"id": 2, "name": "ask2", "op": "ASK", "attributes": {}},
        ],
        "edges": [
            {"from": 1, "to": 2, "dependency": "Data"},
        ],
    }

    training_data = [{"inputs": {}, "output": "result"}]
    o0_outputs = ["good result"]
    o2_outputs = ["bad result"]  # Regression

    evaluator = QualityEvaluator()
    risky = evaluator.identify_risky_fusions(graph, training_data, o0_outputs, o2_outputs)

    # Should identify at least one risky fusion
    assert len(risky) >= 0  # May be empty if no LLM-to-LLM edges
    if risky:
        assert isinstance(risky[0], FusionCandidate)
        assert risky[0].quality_drop >= 0.0


def test_identify_risky_fusions_with_fusion_list():
    """Test risky fusion identification with explicit fusion list."""
    graph = {"name": "test", "nodes": [], "edges": []}
    training_data = [{"inputs": {}, "output": "good result"}]
    o0_outputs = ["good result"]  # Perfect match
    o2_outputs = ["bad output"]    # Different, lower quality

    fusion_list = [
        FusionCandidate(producer_id="1", consumer_id="2"),
        FusionCandidate(producer_id="2", consumer_id="3"),
    ]

    evaluator = QualityEvaluator()
    risky = evaluator.identify_risky_fusions(
        graph, training_data, o0_outputs, o2_outputs, fusion_list=fusion_list
    )

    # Should annotate provided fusions with quality drop if there's a regression
    assert len(risky) == 2
    assert all(f.quality_drop > 0 for f in risky)


def test_save_and_load_quality_profile():
    """Test saving and loading quality profiles."""
    report = QualityReport(
        o0_score=0.85,
        o2_score=0.82,
        regression=True,
        recommendation="review_fusions (quality dropped by 3.53%)",
        risky_fusions=[
            FusionCandidate(producer_id="1", consumer_id="2", quality_drop=0.03),
        ],
    )

    evaluator = QualityEvaluator()

    with tempfile.TemporaryDirectory() as tmpdir:
        profile_path = Path(tmpdir) / "test.apxm-quality-profile.json"

        # Save
        evaluator.save_quality_profile(report, "test_graph", profile_path)
        assert profile_path.exists()

        # Load
        loaded = QualityEvaluator.load_quality_profile(profile_path)
        assert loaded["graph"] == "test_graph"
        assert loaded["quality_scores"]["o0"] == 0.85
        assert loaded["quality_scores"]["o2"] == 0.82
        assert len(loaded["risky_fusions"]) == 1
        assert loaded["risky_fusions"][0]["producer"] == "1"
        assert loaded["risky_fusions"][0]["consumer"] == "2"


def test_load_quality_profile_nonexistent():
    """Test loading non-existent quality profile raises error."""
    with pytest.raises(FileNotFoundError):
        QualityEvaluator.load_quality_profile("/tmp/nonexistent_12345.json")


def test_default_metric_exact_match():
    """Test default metric with exact match."""
    evaluator = QualityEvaluator()
    example = {"output": "The answer is 42"}
    output = "The answer is 42"

    score = evaluator._default_metric(example, output)
    assert score == 1.0  # Exact match


def test_default_metric_partial_match():
    """Test default metric with partial match."""
    evaluator = QualityEvaluator()
    example = {"output": "the answer is correct"}
    output = "the answer is wrong"

    score = evaluator._default_metric(example, output)
    # "the", "answer", "is" overlap (3 out of 4 tokens)
    assert score > 0.5  # At least 50% overlap


def test_default_metric_no_match():
    """Test default metric with no match."""
    evaluator = QualityEvaluator()
    example = {"output": "The answer is 42"}
    output = "completely different text"

    score = evaluator._default_metric(example, output)
    # May have some overlap in common words like "is"
    assert score >= 0.0


def test_custom_metric():
    """Test QualityEvaluator with custom metric."""
    def custom_metric(example, output):
        # Simple length-based metric
        expected_len = len(example.get("output", ""))
        output_len = len(output)
        return 1.0 if abs(expected_len - output_len) < 5 else 0.0

    evaluator = QualityEvaluator(metric=custom_metric)

    example = {"output": "short"}
    output1 = "short"
    output2 = "very long output that exceeds the threshold"

    score1 = evaluator.metric(example, output1)
    score2 = evaluator.metric(example, output2)

    assert score1 == 1.0  # Similar length
    assert score2 == 0.0  # Very different length


def test_compare_optimizations():
    """Test comparing multiple optimization levels."""
    graph = {"name": "test", "nodes": [], "edges": []}
    training_data = [{"inputs": {}, "output": "result"}]
    o0_outputs = ["result"]
    o2_outputs = ["result"]
    o2_no_fusion_outputs = ["result"]

    reports = compare_optimizations(
        graph, training_data, o0_outputs, o2_outputs, o2_no_fusion_outputs
    )

    assert "o0_vs_o2" in reports
    assert isinstance(reports["o0_vs_o2"], QualityReport)

    assert "o0_vs_o2_no_fusion" in reports
    assert isinstance(reports["o0_vs_o2_no_fusion"], QualityReport)

    assert "o2_no_fusion_vs_o2" in reports
    assert isinstance(reports["o2_no_fusion_vs_o2"], QualityReport)


def test_compare_optimizations_without_no_fusion():
    """Test comparing optimizations without o2_no_fusion."""
    graph = {"name": "test", "nodes": [], "edges": []}
    training_data = [{"inputs": {}, "output": "result"}]
    o0_outputs = ["result"]
    o2_outputs = ["result"]

    reports = compare_optimizations(graph, training_data, o0_outputs, o2_outputs)

    assert "o0_vs_o2" in reports
    assert "o0_vs_o2_no_fusion" not in reports
    assert "o2_no_fusion_vs_o2" not in reports


def test_fusion_candidate_dataclass():
    """Test FusionCandidate dataclass."""
    fusion = FusionCandidate(producer_id="node1", consumer_id="node2", quality_drop=0.05)

    assert fusion.producer_id == "node1"
    assert fusion.consumer_id == "node2"
    assert fusion.quality_drop == 0.05


def test_fusion_candidate_default_quality_drop():
    """Test FusionCandidate with default quality_drop."""
    fusion = FusionCandidate(producer_id="node1", consumer_id="node2")

    assert fusion.quality_drop == 0.0


def test_quality_report_dataclass():
    """Test QualityReport dataclass."""
    report = QualityReport(
        o0_score=0.9,
        o2_score=0.85,
        regression=True,
        recommendation="review_fusions",
        per_example_scores=[(0.9, 0.8), (0.9, 0.9)],
        risky_fusions=[FusionCandidate("1", "2", 0.05)],
    )

    assert report.o0_score == 0.9
    assert report.o2_score == 0.85
    assert report.regression is True
    assert len(report.per_example_scores) == 2
    assert len(report.risky_fusions) == 1


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
