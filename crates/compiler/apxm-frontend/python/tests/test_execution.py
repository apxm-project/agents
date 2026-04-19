"""Test execution result types and helpers."""

import json
import pytest


def test_execution_result_from_response():
    """Test ExecutionResult.from_response() with a typical server response."""
    from apxm.execution import ExecutionResult

    data = {
        "content": "Hello, world!",
        "results": {"0": "Hello, world!"},
        "stats": {
            "executed_nodes": 3,
            "failed_nodes": 0,
            "duration_ms": 1234,
        },
        "llm_usage": {
            "input_tokens": 100,
            "output_tokens": 200,
            "total_requests": 2,
        },
    }

    result = ExecutionResult.from_response(data)

    assert result.content == "Hello, world!"
    assert result.results == {"0": "Hello, world!"}
    assert result.stats.executed_nodes == 3
    assert result.stats.failed_nodes == 0
    assert result.stats.duration_ms == 1234
    assert result.llm_usage.input_tokens == 100
    assert result.llm_usage.output_tokens == 200
    assert result.llm_usage.total_requests == 2


def test_execution_result_from_empty_response():
    """Test ExecutionResult.from_response() with minimal data."""
    from apxm.execution import ExecutionResult

    result = ExecutionResult.from_response({})

    assert result.content is None
    assert result.results == {}
    assert result.stats.executed_nodes == 0
    assert result.llm_usage.input_tokens == 0


def test_execution_result_defaults():
    """Test ExecutionResult default values."""
    from apxm.execution import ExecutionResult

    result = ExecutionResult()

    assert result.content is None
    assert result.results == {}
    assert result.stats.executed_nodes == 0
    assert result.llm_usage.total_requests == 0


def test_new_session_returns_string():
    """Test new_session() returns a valid session ID string."""
    from apxm.execution import new_session

    session = new_session()
    assert isinstance(session, str)
    assert session.startswith("s-")
    assert len(session) > 5


def test_new_session_unique():
    """Test new_session() returns unique IDs."""
    from apxm.execution import new_session

    sessions = {new_session() for _ in range(100)}
    assert len(sessions) == 100


def test_error_hierarchy():
    """Test error class hierarchy."""
    from apxm.errors import ApxmError, CompilationError, ExecutionError, ServerError

    assert issubclass(CompilationError, ApxmError)
    assert issubclass(ExecutionError, ApxmError)
    assert issubclass(ServerError, ApxmError)

    # All are also Exception subclasses
    assert issubclass(ApxmError, Exception)


def test_compiled_flow_build_request():
    """Test CompiledFlow._build_request() produces correct structure."""
    from apxm import GraphRecorder
    from apxm.execution import CompiledFlow

    g = GraphRecorder("test_flow")
    g.ask(name="step1", prompt="Do something")
    graph = g.to_graph()

    flow = CompiledFlow(graph)
    request = flow._build_request(("arg1", "arg2"), session_id="test-session")

    assert request["graph"]["name"] == "test_flow"
    assert request["args"] == ["arg1", "arg2"]
    assert request["session_id"] == "test-session"


def test_compiled_flow_build_request_no_session():
    """Test _build_request without session_id."""
    from apxm import GraphRecorder
    from apxm.execution import CompiledFlow

    g = GraphRecorder("test_flow")
    g.ask(name="step1", prompt="Do something")
    graph = g.to_graph()

    flow = CompiledFlow(graph)
    request = flow._build_request((), session_id=None)

    assert "session_id" not in request
    assert request["args"] == []


def test_run_wrapper():
    """Test apxm.run() wrapper."""
    import asyncio
    from apxm.execution import run

    async def simple():
        return 42

    result = run(simple())
    assert result == 42


def test_compiled_flow_save_load_roundtrip():
    """Test save/load roundtrip preserves graph."""
    import tempfile
    from pathlib import Path
    from apxm import GraphRecorder
    from apxm.execution import CompiledFlow

    g = GraphRecorder("roundtrip_test")
    g.param("topic", "str")
    g.ask(name="step1", prompt="Research {0}")
    graph = g.to_graph()

    flow = CompiledFlow(graph)

    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as tmp:
        tmp_path = tmp.name

    try:
        flow.save(tmp_path)
        loaded = CompiledFlow.load(tmp_path)
        assert loaded._graph.name == "roundtrip_test"
        assert len(loaded._graph.nodes) == 1
        assert len(loaded._graph.parameters) == 1
    finally:
        Path(tmp_path).unlink(missing_ok=True)
