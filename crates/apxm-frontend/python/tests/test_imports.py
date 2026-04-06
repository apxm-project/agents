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
