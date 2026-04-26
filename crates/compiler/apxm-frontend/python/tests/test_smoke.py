"""Smoke tests for the flattened APXM Python package."""

from apxm import ApxmGraph, GraphRecorder, WorkflowTargetKind, compile
from apxm.constants import OP_ASK, OP_WORKFLOW_SPAWN, TEMPLATE_STR


def test_public_imports_and_all():
    import apxm

    required_exports = {
        "ApxmGraph",
        "GraphNode",
        "GraphEdge",
        "Parameter",
        "GraphRecorder",
        "NodeRef",
        "compile",
        "CompiledFlow",
        "ExecutionMode",
        "AgentConfig",
        "ToolsConfig",
        "ApxmError",
        "CompilationError",
        "ExecutionError",
        "ExecutionResult",
        "ExecutionStats",
        "FlowModule",
        "LLMUsage",
        "ServerError",
        "close",
        "new_session",
        "run",
        "WorkflowTargetKind",
    }

    assert ApxmGraph is apxm.ApxmGraph
    assert GraphRecorder is apxm.GraphRecorder
    assert compile is apxm.compile
    assert required_exports.issubset(set(apxm.__all__))


def test_generated_constants_import():
    import apxm.constants as public_constants
    from apxm._generated.constants import (
        AWAIT_RESULT,
        MODEL,
        SESSION_ROOT,
        TARGET_KIND,
        TOOL_GROUPS,
        WORKFLOW_TARGET_KIND_GRAPH_PATH,
    )

    assert MODEL == public_constants.MODEL
    assert TOOL_GROUPS == public_constants.TOOL_GROUPS
    assert TARGET_KIND == public_constants.TARGET_KIND
    assert SESSION_ROOT == public_constants.SESSION_ROOT
    assert AWAIT_RESULT == public_constants.AWAIT_RESULT
    assert WORKFLOW_TARGET_KIND_GRAPH_PATH == WorkflowTargetKind.GRAPH_PATH.value


def test_generated_operations_import():
    from apxm._generated.operations import ASK, WORKFLOW_SPAWN

    assert ASK.op == OP_ASK
    assert WORKFLOW_SPAWN.op == OP_WORKFLOW_SPAWN


def test_generated_agents_import():
    from apxm._generated.agents import ALL_AGENTS, claude

    assert claude in ALL_AGENTS


def test_graph_recorder_to_graph():
    recorder = GraphRecorder("smoke_graph")
    ask_node = recorder.ask(name="ask_question", prompt="What is APXM?")

    graph = recorder.to_graph()

    assert graph.name == "smoke_graph"
    assert len(graph.nodes) == 1
    assert len(graph.edges) == 0
    assert graph.nodes[0].name == "ask_question"
    assert graph.nodes[0].op == OP_ASK
    assert graph.nodes[0].attributes[TEMPLATE_STR] == "What is APXM?"
    assert ask_node.name == "ask_question"


def test_apxm_graph_to_air_produces_output():
    recorder = GraphRecorder("smoke_air")
    recorder.ask(name="ask_question", prompt="What is APXM?")

    graph = recorder.to_graph()
    air = graph.to_air()

    assert isinstance(graph, ApxmGraph)
    assert isinstance(air, str)
    assert air.strip()
    assert "module {" in air
    assert "func.func @smoke_air" in air
