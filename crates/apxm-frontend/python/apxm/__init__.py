"""Python frontend for APXM workflows."""

from __future__ import annotations

# Re-export core graph types and functions
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
from .proxy import GraphRecorder, NodeRef
from .sugar import AgentHandle, Team

# Re-export providers module
from .providers import ProviderSpec, list_providers, resolve_provider

# Re-export generated modules
from . import _generated

__all__ = [
    # Submodules
    "_generated",
    # Graph types
    "AgentConfig",
    "AgentHandle",
    "ApxmGraph",
    "BashConfig",
    "CompiledFlow",
    "ExecutionMode",
    "FlowModule",
    "GraphEdge",
    "GraphNode",
    "GraphRecorder",
    "NodeRef",
    "Parameter",
    "ReadConfig",
    "SearchWebConfig",
    "Team",
    "ToolsConfig",
    "WorkflowCheckpoint",
    "WriteConfig",
    # Functions
    "compile",
    "validate_graph",
    # Providers
    "ProviderSpec",
    "list_providers",
    "resolve_provider",
]
