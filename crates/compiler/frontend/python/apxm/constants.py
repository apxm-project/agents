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
# Mirror of crates/machine/contracts/src/constants.rs.
ENV_APXM_PYTHON_TOOLS_OUT = "APXM_PYTHON_TOOLS_OUT"
ENV_APXM_MOCK_BACKEND = "APXM_MOCK_BACKEND"
ENV_APXM_SERVER_URL = "APXM_SERVER_URL"
ENV_FLAG_ENABLED = "1"


class DependencyType(str, Enum):
    DATA = "Data"
    CONTROL = "Control"
    EFFECT = "Effect"


# Mirror of crates/machine/ais/src/attrs.rs::PromptInputRole.
class PromptInputRole(str, Enum):
    USER = "user"
    SYSTEM = "system"
    DEPENDENCY_ONLY = "dependency_only"
    TOOL_CONTEXT = "tool_context"
    CONTROL = "control"


class ToolGroup(str, Enum):
    FILE = "file"
    FILE_READ = "file:read"
    FILE_WRITE = "file:write"
    HTTP = "http"
    WEB = "web"
    SEARCH = "search"
    WEB_SEARCH = "web:search"
    SKILLS = "skills"
    AUTHORING = "authoring"
    TASK = "task"
    AGENT_MANAGEMENT = "agent_management"


class Capability(str, Enum):
    BASH = "bash"
    READ = "read"
    WRITE = "write"
    SEARCH_WEB = "search_web"
    HTTP_GET = "http_get"
    HTTP_POST = "http_post"
    SEARCH_SKILLS = "search_skills"
    SCHEDULE = "schedule"
    MANAGE_TASK = "manage_task"


def normalize_dependency_type(value: DependencyType | str) -> str:
    if isinstance(value, DependencyType):
        return value.value
    if isinstance(value, str):
        return value
    raise TypeError("dependency must be a DependencyType or string")


def normalize_prompt_input_role(value: PromptInputRole | str) -> str:
    if isinstance(value, PromptInputRole):
        return value.value
    if isinstance(value, str) and value in {role.value for role in PromptInputRole}:
        return value
    allowed = ", ".join(role.value for role in PromptInputRole)
    raise ValueError(f"invalid prompt input role {value!r}; expected one of: {allowed}")


def normalize_tool_group(value: ToolGroup | str) -> str:
    if isinstance(value, ToolGroup):
        return value.value
    if isinstance(value, str):
        return value
    raise TypeError("tool group must be a ToolGroup or string")


def normalize_capability(value: Capability | str) -> str:
    if isinstance(value, Capability):
        return value.value
    if isinstance(value, str):
        return value
    raise TypeError("capability must be a Capability or string")


DEPENDENCY_DATA = DependencyType.DATA.value
DEPENDENCY_CONTROL = DependencyType.CONTROL.value
DEPENDENCY_EFFECT = DependencyType.EFFECT.value

TOOL_GROUP_WEB = ToolGroup.WEB.value
TOOL_GROUP_FILE = ToolGroup.FILE.value
TOOL_GROUP_FILE_READ = ToolGroup.FILE_READ.value
TOOL_GROUP_FILE_WRITE = ToolGroup.FILE_WRITE.value
TOOL_GROUP_HTTP = ToolGroup.HTTP.value
TOOL_GROUP_SEARCH = ToolGroup.SEARCH.value
TOOL_GROUP_WEB_SEARCH = ToolGroup.WEB_SEARCH.value
TOOL_GROUP_SKILLS = ToolGroup.SKILLS.value
TOOL_GROUP_AUTHORING = ToolGroup.AUTHORING.value
TOOL_GROUP_TASK = ToolGroup.TASK.value
TOOL_GROUP_AGENT_MANAGEMENT = ToolGroup.AGENT_MANAGEMENT.value

CAPABILITY_BASH = Capability.BASH.value
CAPABILITY_READ = Capability.READ.value
CAPABILITY_WRITE = Capability.WRITE.value
CAPABILITY_SEARCH_WEB = Capability.SEARCH_WEB.value
CAPABILITY_HTTP_GET = Capability.HTTP_GET.value
CAPABILITY_HTTP_POST = Capability.HTTP_POST.value
CAPABILITY_SEARCH_SKILLS = Capability.SEARCH_SKILLS.value
CAPABILITY_SCHEDULE = Capability.SCHEDULE.value
CAPABILITY_MANAGE_TASK = Capability.MANAGE_TASK.value

COMMUNICATE_PROTOCOL_ACP = "acp"

PYTHON_TOOL_MANIFEST_HANDLER_ID = "handler_id"
PYTHON_TOOL_MANIFEST_MODULE = "module"
PYTHON_TOOL_MANIFEST_QUALNAME = "qualname"
PYTHON_TOOL_MANIFEST_NAME = "name"
PYTHON_TOOL_MANIFEST_DESCRIPTION = "description"
PYTHON_TOOL_MANIFEST_SCHEMA = "schema"

# Mirror of crates/machine/ais/src/attrs.rs.
INPUT_ROLES = "input_roles"

_g = globals()
for _spec in _ops.ALL_OPERATIONS:
    _g[f"OP_{_spec.op}"] = _spec.op
del _g, _spec, _ops
