"""Python frontend for APXM workflows."""

from __future__ import annotations

from . import _generated
from .agent import Agent, AgentHooks, BoundAgent
try:
    from ._generated.models import Anthropic, Google, ModelId, OpenAI, Vllm
except ImportError:
    Anthropic = None  # type: ignore[assignment,misc]
    Google = None  # type: ignore[assignment,misc]
    ModelId = None  # type: ignore[assignment,misc]
    OpenAI = None  # type: ignore[assignment,misc]
    Vllm = None  # type: ignore[assignment,misc]
from .config import (
    AgentConfig,
    BashConfig,
    ReadConfig,
    SearchWebConfig,
    ToolsConfig,
    WriteConfig,
)
from .decorators import compile
from .errors import ApxmError, CompilationError, ExecutionError, ServerError
from .execution import (
    CompiledFlow,
    ExecutionMode,
    ExecutionResult,
    ExecutionStats,
    LLMUsage,
    WorkflowCheckpoint,
    close,
    new_session,
    run,
    validate_graph,
)
from .ir import ApxmGraph, GraphEdge, GraphNode, Parameter, load_graph
from .module import FlowModule
from .providers import ProviderSpec, list_providers, resolve_provider
from .proxy import GraphRecorder, NodeRef
from .sugar import AgentHandle, Team
from .tools import FunctionTool, ToolContext, tool

__all__ = [
    "_generated",
    "Agent",
    "AgentConfig",
    "AgentHandle",
    "AgentHooks",
    "Anthropic",
    "ApxmError",
    "ApxmGraph",
    "BashConfig",
    "BoundAgent",
    "close",
    "compile",
    "CompiledFlow",
    "CompilationError",
    "ExecutionError",
    "ExecutionMode",
    "ExecutionResult",
    "ExecutionStats",
    "FlowModule",
    "FunctionTool",
    "Google",
    "GraphEdge",
    "GraphNode",
    "GraphRecorder",
    "LLMUsage",
    "list_providers",
    "load_graph",
    "ModelId",
    "new_session",
    "NodeRef",
    "OpenAI",
    "Parameter",
    "ProviderSpec",
    "ReadConfig",
    "run",
    "resolve_provider",
    "SearchWebConfig",
    "ServerError",
    "Team",
    "tool",
    "ToolContext",
    "ToolsConfig",
    "validate_graph",
    "Vllm",
    "WorkflowCheckpoint",
    "WriteConfig",
]
