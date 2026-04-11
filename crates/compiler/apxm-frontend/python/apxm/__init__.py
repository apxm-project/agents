"""Python frontend for APXM workflows."""

from __future__ import annotations

from . import _generated
try:
    from ._generated.models import Anthropic, Google, ModelId, OpenAI
except ImportError:
    Anthropic = None  # type: ignore[assignment,misc]
    Google = None  # type: ignore[assignment,misc]
    ModelId = None  # type: ignore[assignment,misc]
    OpenAI = None  # type: ignore[assignment,misc]
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
    validate_graph,
)
from .ir import ApxmGraph, GraphEdge, GraphNode, Parameter
from .module import FlowModule
from .providers import ProviderSpec, list_providers, resolve_provider
from .proxy import GraphRecorder, NodeRef
from .sugar import AgentHandle, Team

__all__ = [
    "_generated",
    "AgentConfig",
    "AgentHandle",
    "Anthropic",
    "ApxmError",
    "ApxmGraph",
    "BashConfig",
    "close",
    "compile",
    "CompiledFlow",
    "CompilationError",
    "ExecutionError",
    "ExecutionMode",
    "ExecutionResult",
    "ExecutionStats",
    "FlowModule",
    "Google",
    "GraphEdge",
    "GraphNode",
    "GraphRecorder",
    "LLMUsage",
    "list_providers",
    "ModelId",
    "new_session",
    "NodeRef",
    "OpenAI",
    "Parameter",
    "ProviderSpec",
    "ReadConfig",
    "resolve_provider",
    "SearchWebConfig",
    "ServerError",
    "Team",
    "ToolsConfig",
    "validate_graph",
    "WorkflowCheckpoint",
    "WriteConfig",
]
