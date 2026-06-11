"""Smoke tests for the flattened APXM Python package."""

from apxm import ApxmGraph, FunctionTool, GraphRecorder, ToolContext, compile, tool


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
        "FunctionTool",
        "LLMUsage",
        "ServerError",
        "tool",
        "ToolContext",
        "close",
        "new_session",
        "run",
        "WorkflowTargetKind",
    }

    assert ApxmGraph is apxm.ApxmGraph
    assert FunctionTool is apxm.FunctionTool
    assert GraphRecorder is apxm.GraphRecorder
    assert ToolContext is apxm.ToolContext
    assert compile is apxm.compile
    assert tool is apxm.tool
    assert required_exports.issubset(set(apxm.__all__))
