"""Test that generated imports work correctly."""

def test_import_constants():
    """Verify constants can be imported from _generated."""
    from apxm._generated.constants import (
        AGENT_NAME,
        AWAIT_RESULT,
        MODEL,
        SESSION_ROOT,
        TARGET_KIND,
        TEMPLATE_STR,
        WORKFLOW_TARGET_KIND_GRAPH_PATH,
        WORKFLOW_SPAWN_PATH_TARGET_KINDS,
    )
    assert MODEL == "model"
    assert AGENT_NAME == "agent_name"
    assert TEMPLATE_STR == "template_str"
    assert TARGET_KIND == "target_kind"
    assert SESSION_ROOT == "session_root"
    assert AWAIT_RESULT == "await_result"
    assert WORKFLOW_TARGET_KIND_GRAPH_PATH == "graph_path"
    assert WORKFLOW_TARGET_KIND_GRAPH_PATH in WORKFLOW_SPAWN_PATH_TARGET_KINDS


def test_import_operations():
    """Verify operations can be imported from _generated."""
    from apxm._generated.operations import ASK, SPAWN_AGENT, THINK, WORKFLOW_SPAWN
    assert ASK.op == "ASK"
    assert THINK.op == "THINK"
    assert SPAWN_AGENT.op == "SPAWN_AGENT"
    assert WORKFLOW_SPAWN.op == "WORKFLOW_SPAWN"


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
        WorkflowTargetKind,
        compile,
        AgentHandle,
        Team,
    )
    assert GraphRecorder is not None
    assert NodeRef is not None
    assert ApxmGraph is not None
    assert WorkflowTargetKind.GRAPH_PATH.value is not None
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
    from apxm import run, run_workflow_file
    assert callable(run)
    assert callable(run_workflow_file)


def test_import_execution_result():
    """Verify execution result types can be imported."""
    from apxm import (
        ExecutionResult,
        ExecutionStats,
        LLMUsage,
        WorkflowRunResult,
        new_session,
    )
    assert ExecutionResult is not None
    assert ExecutionStats is not None
    assert LLMUsage is not None
    assert WorkflowRunResult is not None
    assert callable(new_session)


def test_import_load_graph():
    """Verify load_graph and FlowModule can be imported."""
    from apxm import load_graph, FlowModule
    assert callable(load_graph)
    assert FlowModule is not None
