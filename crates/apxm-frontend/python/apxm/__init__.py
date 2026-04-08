"""Python frontend for APXM workflows."""

from __future__ import annotations

from . import _generated
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
from .quality import QualityEvaluator, QualityReport, FusionCandidate, compare_optimizations
from .sugar import AgentHandle, Team

__all__ = [
    "_generated",
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
    "ProviderSpec",
    "ReadConfig",
    "SearchWebConfig",
    "Team",
    "WorkflowCheckpoint",
    "WriteConfig",
    "list_providers",
    "resolve_provider",
    "validate_graph",
    "QualityEvaluator",
    "QualityReport",
    "FusionCandidate",
    "compare_optimizations",
]
