"""
Tests for DSPy bridge module.

These tests verify:
1. DSPy bridge can be imported (even without DSPy installed)
2. Graph optimization works with stub mode
3. Training data conversion works
4. Template extraction works
"""

import json
import tempfile
from pathlib import Path

import pytest

from apxm.dspy_bridge import (
    ApxmDspyBridge,
    OptimizationConfig,
    load_training_data,
    profile_to_examples,
)


def test_import():
    """Test that DSPy bridge can be imported."""
    assert ApxmDspyBridge is not None
    assert OptimizationConfig is not None


def test_bridge_initialization_without_config():
    """Test bridge can be initialized without LM config."""
    bridge = ApxmDspyBridge()
    assert bridge is not None


def test_bridge_initialization_with_config():
    """Test bridge can be initialized with LM config."""
    config = {
        "model": "gpt-4o-mini",
        "max_tokens": 2048,
    }
    bridge = ApxmDspyBridge(lm_config=config)
    assert bridge is not None


def test_optimize_graph_no_llm_nodes():
    """Test optimizing a graph with no LLM nodes."""
    graph = {
        "name": "test_graph",
        "nodes": [
            {"id": 1, "name": "const", "op": "CONST_STR", "attributes": {}},
        ],
        "edges": [],
    }

    bridge = ApxmDspyBridge()
    result = bridge.optimize_graph(graph)

    assert result == graph  # Should return unchanged


def test_optimize_graph_with_llm_nodes_no_training_data():
    """Test optimizing graph with LLM nodes but no training data."""
    graph = {
        "name": "test_graph",
        "nodes": [
            {
                "id": 1,
                "name": "ask_node",
                "op": "ASK",
                "attributes": {"template_str": "Answer: {{question}}"},
            },
        ],
        "edges": [],
    }

    bridge = ApxmDspyBridge()
    result = bridge.optimize_graph(graph, training_data=None)

    # Should return unchanged when no training data
    assert result == graph


def test_optimize_graph_stub_mode():
    """Test graph optimization in stub mode (no actual DSPy)."""
    graph = {
        "name": "test_graph",
        "nodes": [
            {
                "id": 1,
                "name": "ask_node",
                "op": "ASK",
                "attributes": {"template_str": "Answer: {{question}}"},
            },
        ],
        "edges": [],
    }

    training_data = [
        {"inputs": {"question": "What is 2+2?"}, "output": "4"},
        {"inputs": {"question": "What is Python?"}, "output": "A language"},
    ]

    bridge = ApxmDspyBridge(lm_config=None)  # Stub mode
    result = bridge.optimize_graph(graph, training_data=training_data)

    # In stub mode, should return graph unchanged
    assert result["name"] == graph["name"]
    assert len(result["nodes"]) == len(graph["nodes"])


def test_optimize_template_stub_mode():
    """Test template optimization in stub mode."""
    template = "Answer the question: {{question}}"
    examples = [
        {"inputs": {"question": "What is 2+2?"}, "output": "4"},
    ]

    bridge = ApxmDspyBridge(lm_config=None)
    result = bridge.optimize_template(template, examples)

    # In stub mode (lm=None), DSPy can still extract few-shot examples from labeled data
    # The optimized prompt should include the examples and the question placeholder
    assert "{{question}}" in result  # Variable placeholder should be in result
    assert "Example" in result or "question:" in result  # Should have examples or structure


def test_load_training_data_from_list():
    """Test loading training data from JSON file (list format)."""
    data = [
        {"inputs": {"x": "1"}, "output": "2"},
        {"inputs": {"x": "3"}, "output": "4"},
    ]

    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
        json.dump(data, f)
        temp_path = f.name

    try:
        result = load_training_data(temp_path)
        assert len(result) == 2
        assert result[0]["inputs"]["x"] == "1"
        assert result[1]["output"] == "4"
    finally:
        Path(temp_path).unlink()


def test_load_training_data_from_dict():
    """Test loading training data from JSON file (per-node format)."""
    data = {
        "node_1": [
            {"inputs": {"x": "1"}, "output": "2"},
        ],
        "node_2": [
            {"inputs": {"x": "3"}, "output": "4"},
        ],
    }

    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False) as f:
        json.dump(data, f)
        temp_path = f.name

    try:
        result = load_training_data(temp_path)
        assert len(result) == 2
        # Should flatten all node examples
    finally:
        Path(temp_path).unlink()


def test_load_training_data_nonexistent_file():
    """Test loading from non-existent file."""
    result = load_training_data("/tmp/nonexistent_file_12345.json")
    assert result == []


def test_profile_to_examples_nonexistent_session():
    """Test converting non-existent session to examples."""
    result = profile_to_examples("/tmp/nonexistent_session_12345")
    assert result == []


def test_profile_to_examples_with_mock_data():
    """Test converting session results to examples."""
    session_data = {
        "1": {
            "success": True,
            "inputs": {"question": "What is 2+2?"},
            "output": "4",
        },
        "2": {
            "success": False,  # Should be filtered out
            "inputs": {"question": "Bad question"},
            "output": "Error",
        },
        "3": {
            "success": True,
            "inputs": {"context": "Paris is the capital of France"},
            "output": "The capital is Paris",
        },
    }

    with tempfile.TemporaryDirectory() as temp_dir:
        session_path = Path(temp_dir)
        results_file = session_path / "results.json"

        with open(results_file, "w") as f:
            json.dump(session_data, f)

        result = profile_to_examples(session_path)

        # Should only include successful executions
        assert len(result) == 2
        assert result[0]["output"] == "4"
        assert result[1]["output"] == "The capital is Paris"


def test_optimization_config_defaults():
    """Test OptimizationConfig default values."""
    config = OptimizationConfig()

    assert config.optimizer == "bootstrap_fewshot"
    assert config.max_bootstrapped_demos == 5
    assert config.max_labeled_demos == 10
    assert config.num_trials == 10
    assert config.metric_threshold == 0.5
    assert config.verbose is False


def test_optimization_config_custom():
    """Test OptimizationConfig with custom values."""
    config = OptimizationConfig(
        optimizer="mipro_v2",
        max_bootstrapped_demos=10,
        verbose=True,
    )

    assert config.optimizer == "mipro_v2"
    assert config.max_bootstrapped_demos == 10
    assert config.verbose is True


def test_graph_with_multiple_llm_nodes():
    """Test graph with multiple ASK/THINK/REASON nodes."""
    graph = {
        "name": "multi_node_graph",
        "nodes": [
            {
                "id": 1,
                "name": "ask",
                "op": "ASK",
                "attributes": {"template_str": "Q: {{question}}"},
            },
            {
                "id": 2,
                "name": "think",
                "op": "THINK",
                "attributes": {"template_str": "Think: {{context}}"},
            },
            {
                "id": 3,
                "name": "const",
                "op": "CONST_STR",
                "attributes": {},
            },
            {
                "id": 4,
                "name": "reason",
                "op": "REASON",
                "attributes": {"template_str": "Reason: {{input}}"},
            },
        ],
        "edges": [],
    }

    training_data = [
        {"inputs": {"question": "test"}, "output": "answer"},
    ]

    bridge = ApxmDspyBridge(lm_config=None)
    result = bridge.optimize_graph(graph, training_data=training_data)

    # Should process 3 LLM nodes (ASK, THINK, REASON)
    llm_nodes = [n for n in result["nodes"] if n["op"] in {"ASK", "THINK", "REASON"}]
    assert len(llm_nodes) == 3


def test_template_without_variables():
    """Test optimizing template without any {{variables}}."""
    template = "This is a static prompt with no variables"
    examples = [
        {"inputs": {}, "output": "result"},
    ]

    bridge = ApxmDspyBridge(lm_config=None)
    result = bridge.optimize_template(template, examples)

    # Should return original when no variables found
    assert result == template


def test_real_optimization_with_demos():
    """Test that DSPy actually extracts few-shot demos."""
    template = "Classify sentiment: {{text}}"
    examples = [
        {"inputs": {"text": "This is amazing!"}, "output": "positive"},
        {"inputs": {"text": "This is terrible."}, "output": "negative"},
        {"inputs": {"text": "It's okay."}, "output": "neutral"},
    ]

    bridge = ApxmDspyBridge(lm_config=None)
    config = OptimizationConfig(
        max_labeled_demos=3,
        max_bootstrapped_demos=0,  # Only use labeled demos
        verbose=False,
    )

    result = bridge.optimize_template(template, examples, config)

    # Optimized prompt should include few-shot examples
    assert "Example" in result
    assert "{{text}}" in result  # Should preserve the variable placeholder
    # Should include some of the training examples
    assert ("amazing" in result or "terrible" in result or "okay" in result)


def test_extract_optimized_prompt():
    """Test the _extract_optimized_prompt helper."""
    bridge = ApxmDspyBridge(lm_config=None)

    # Create a mock predictor with demos
    class MockPredictor:
        def __init__(self):
            self.demos = []
            self.extended_signature = None

    # Create a mock demo
    class MockDemo:
        def __init__(self):
            self.question = "What is 2+2?"
            self.answer = "4"

    predictor = MockPredictor()
    predictor.demos = [MockDemo()]

    original_template = "Answer: {{question}}"
    input_vars = ["question"]

    result = bridge._extract_optimized_prompt(predictor, original_template, input_vars)

    # Should include the demo
    assert "Example" in result
    assert "What is 2+2?" in result
    assert "{{question}}" in result  # Should preserve placeholder


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
