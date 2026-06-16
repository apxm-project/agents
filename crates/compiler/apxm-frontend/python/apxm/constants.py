"""Graph constants for APXM Python frontend.

Re-exports attribute constants, operation constants, category constants,
and LLM_OPS from the generated modules.
"""

from __future__ import annotations

from enum import Enum

from apxm._generated.constants import *  # noqa: F401, F403
from apxm._generated.operations import (  # noqa: F401
    CATEGORY_COMMUNICATION,
    CATEGORY_CONTROL_FLOW,
    CATEGORY_COORDINATION,
    CATEGORY_ERROR_HANDLING,
    CATEGORY_IDENTITY,
    CATEGORY_INTERNAL,
    CATEGORY_MEMORY,
    CATEGORY_METADATA,
    CATEGORY_REASONING,
    CATEGORY_SYNCHRONIZATION,
    CATEGORY_TOOLS,
    LLM_OPS,
)

from apxm._generated import operations as _ops

ENV_APXM_EMIT_AIR = "APXM_EMIT_AIR"
ENV_APXM_CONFIG = "APXM_CONFIG"
ENV_APXM_HOME = "APXM_HOME"
ENV_APXM_BIN = "APXM_BIN"
ENV_APXM_MOCK_BACKEND = "APXM_MOCK_BACKEND"
ENV_APXM_SERVER_URL = "APXM_SERVER_URL"
ENV_FLAG_ENABLED = "1"


class DependencyType(str, Enum):
    DATA = "Data"
    CONTROL = "Control"
    EFFECT = "Effect"


class ToolGroup(str, Enum):
    WEB = "web"
    SKILLS = "skills"
    AUTHORING = "authoring"


def normalize_dependency_type(value: DependencyType | str) -> str:
    if isinstance(value, DependencyType):
        return value.value
    if isinstance(value, str):
        return value
    raise TypeError("dependency must be a DependencyType or string")


def normalize_tool_group(value: ToolGroup | str) -> str:
    if isinstance(value, ToolGroup):
        return value.value
    if isinstance(value, str):
        return value
    raise TypeError("tool group must be a ToolGroup or string")


DEPENDENCY_DATA = DependencyType.DATA.value
DEPENDENCY_CONTROL = DependencyType.CONTROL.value
DEPENDENCY_EFFECT = DependencyType.EFFECT.value

TOOL_GROUP_WEB = ToolGroup.WEB.value
TOOL_GROUP_SKILLS = ToolGroup.SKILLS.value
TOOL_GROUP_AUTHORING = ToolGroup.AUTHORING.value

CAPABILITY_SEARCH_SKILLS = "search_skills"

COMMUNICATE_PROTOCOL_ACP = "acp"

PYTHON_TOOL_MANIFEST_HANDLER_ID = "handler_id"
PYTHON_TOOL_MANIFEST_MODULE = "module"
PYTHON_TOOL_MANIFEST_QUALNAME = "qualname"
PYTHON_TOOL_MANIFEST_NAME = "name"
PYTHON_TOOL_MANIFEST_DESCRIPTION = "description"
PYTHON_TOOL_MANIFEST_SCHEMA = "schema"
PYTHON_TOOL_MANIFEST_SOURCE_FILE = "source_file"
PYTHON_TOOLS_AIR_COMMENT_PREFIX = "; __apxm_python_tools__ "

# Reserved `input_names` entry carrying the system prompt as a dataflow value.
# Mirrors `apxm-core` `graph::attrs::SYSTEM_PROMPT_INPUT`; when an ASK operand is
# bound to this name the runtime uses it as the system prompt (FR-005/FR-009).
SYSTEM_PROMPT_INPUT = "__system"

_g = globals()
for _spec in _ops.ALL_OPERATIONS:
    _g[f"OP_{_spec.op}"] = _spec.op
del _g, _spec, _ops
