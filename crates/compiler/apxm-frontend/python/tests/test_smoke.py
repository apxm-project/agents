"""Smoke tests for the flattened APXM Python package."""

from apxm import ApxmGraph, GraphRecorder, compile


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
        "LLMUsage",
        "ServerError",
        "close",
        "new_session",
    }

    assert ApxmGraph is apxm.ApxmGraph
    assert GraphRecorder is apxm.GraphRecorder
    assert compile is apxm.compile
    assert required_exports.issubset(set(apxm.__all__))


def test_generated_constants_import():
    from apxm._generated.constants import MODEL

    assert MODEL == "model"


def test_generated_operations_import():
    from apxm._generated.operations import ASK

    assert ASK.op == "ASK"


def test_generated_agents_import():
    from apxm._generated.agents import claude

    assert claude.name == "claude"


def test_graph_recorder_to_graph():
    recorder = GraphRecorder("smoke_graph")
    ask_node = recorder.ask("ask_question", "What is APXM?")

    graph = recorder.to_graph()

    assert graph.name == "smoke_graph"
    assert len(graph.nodes) == 1
    assert len(graph.edges) == 0
    assert graph.nodes[0].name == "ask_question"
    assert graph.nodes[0].op == "ASK"
    assert graph.nodes[0].attributes["template_str"] == "What is APXM?"
    assert ask_node.name == "ask_question"


def test_apxm_graph_to_air_produces_output():
    recorder = GraphRecorder("smoke_air")
    recorder.ask("ask_question", "What is APXM?")

    graph = recorder.to_graph()
    air = graph.to_air()

    assert isinstance(graph, ApxmGraph)
    assert isinstance(air, str)
    assert air.strip()
    assert "module {" in air
    assert "func.func @smoke_air" in air
