"""Test that generated imports work correctly."""

def test_import_constants():
    """Verify constants can be imported from _generated."""
    from apxm._generated.constants import MODEL, AGENT_NAME, TEMPLATE_STR
    assert MODEL == "model"
    assert AGENT_NAME == "agent_name"
    assert TEMPLATE_STR == "template_str"


def test_import_operations():
    """Verify operations can be imported from _generated."""
    from apxm._generated.operations import ASK, THINK, SPAWN_AGENT
    assert ASK.op == "ASK"
    assert THINK.op == "THINK"
    assert SPAWN_AGENT.op == "SPAWN_AGENT"


def test_import_agents():
    """Verify agents can be imported from _generated."""
    from apxm._generated.agents import claude
    assert claude.name == "claude"


def test_graph_imports():
    """Verify graph module exports work."""
    from apxm import (
        GraphRecorder,
        NodeRef,
        ApxmGraph,
        compile,
        AgentHandle,
        Team,
    )
    assert GraphRecorder is not None
    assert NodeRef is not None
    assert ApxmGraph is not None
    assert compile is not None
    assert AgentHandle is not None
    assert Team is not None


def test_import_error_types():
    """Verify error types can be imported."""
    from apxm import ApxmError, CompilationError, ExecutionError, ServerError
    assert issubclass(CompilationError, ApxmError)
    assert issubclass(ExecutionError, ApxmError)
    assert issubclass(ServerError, ApxmError)


def test_import_run():
    """Verify run() can be imported."""
    from apxm import run
    assert callable(run)


def test_import_execution_result():
    """Verify execution result types can be imported."""
    from apxm import ExecutionResult, ExecutionStats, LLMUsage, new_session
    assert ExecutionResult is not None
    assert ExecutionStats is not None
    assert LLMUsage is not None
    assert callable(new_session)


def test_import_load_graph():
    """Verify load_graph and FlowModule can be imported."""
    from apxm import load_graph, FlowModule
    assert callable(load_graph)
    assert FlowModule is not None
