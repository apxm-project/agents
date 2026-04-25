"""Python frontend for APXM workflows."""

from __future__ import annotations

from . import _generated
from .agent import Agent, AgentHooks, BoundAgent
from .backends import (
    BackendRegistryError,
    BackendRoute,
    RegisteredBackend,
    RegisteredModel,
    config_path,
    get_backend,
    list_backends,
    select,
    select_backend,
)
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
    ExecutionOptions,
    HookConfig,
    HookEvent,
    LoopGuardMiddlewareConfig,
    MiddlewareKind,
    NodePolicy,
    ReadConfig,
    SearchDepth,
    SearchWebConfig,
    TimeoutMiddlewareConfig,
    ToolsConfig,
    WorkflowTargetKind,
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
    WorkflowRunResult,
    WorkflowCheckpoint,
    close,
    new_session,
    run,
    run_workflow_file,
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
    "BackendRegistryError",
    "BackendRoute",
    "BashConfig",
    "BoundAgent",
    "close",
    "compile",
    "CompiledFlow",
    "CompilationError",
    "ExecutionError",
    "ExecutionOptions",
    "ExecutionMode",
    "ExecutionResult",
    "ExecutionStats",
    "FlowModule",
    "FunctionTool",
    "Google",
    "GraphEdge",
    "GraphNode",
    "GraphRecorder",
    "HookConfig",
    "HookEvent",
    "LLMUsage",
    "config_path",
    "get_backend",
    "list_providers",
    "list_backends",
    "LoopGuardMiddlewareConfig",
    "load_graph",
    "MiddlewareKind",
    "ModelId",
    "new_session",
    "NodePolicy",
    "NodeRef",
    "OpenAI",
    "Parameter",
    "ProviderSpec",
    "ReadConfig",
    "RegisteredBackend",
    "RegisteredModel",
    "run",
    "resolve_provider",
    "SearchWebConfig",
    "SearchDepth",
    "ServerError",
    "select",
    "select_backend",
    "Team",
    "TimeoutMiddlewareConfig",
    "tool",
    "ToolContext",
    "ToolsConfig",
    "validate_graph",
    "Vllm",
    "WorkflowTargetKind",
    "WorkflowCheckpoint",
    "WorkflowRunResult",
    "WriteConfig",
    "run_workflow_file",
]
