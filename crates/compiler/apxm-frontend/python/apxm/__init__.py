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
from .execution import CompiledFlow, ExecutionMode, WorkflowCheckpoint, validate_graph
from .ir import ApxmGraph, GraphEdge, GraphNode, Parameter
from .module import FlowModule
from .providers import ProviderSpec, list_providers, resolve_provider
from .proxy import GraphRecorder, NodeRef
from .sugar import AgentHandle, Team

__all__ = [
    "_generated",
    "Anthropic",
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
    "AgentHandle",
    "BashConfig",
    "FlowModule",
    "Google",
    "ModelId",
    "OpenAI",
    "ProviderSpec",
    "ReadConfig",
    "SearchWebConfig",
    "Team",
    "WorkflowCheckpoint",
    "WriteConfig",
    "list_providers",
    "resolve_provider",
    "validate_graph",
]
