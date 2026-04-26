"""Graph constants for APXM Python frontend.

Re-exports attribute constants, operation constants, category constants,
and LLM_OPS from the generated modules.
"""

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

DEPENDENCY_DATA = "Data"
DEPENDENCY_CONTROL = "Control"
DEPENDENCY_EFFECT = "Effect"

COMMUNICATE_PROTOCOL_ACP = "acp"

PYTHON_TOOL_MANIFEST_HANDLER_ID = "handler_id"
PYTHON_TOOL_MANIFEST_MODULE = "module"
PYTHON_TOOL_MANIFEST_QUALNAME = "qualname"
PYTHON_TOOL_MANIFEST_NAME = "name"
PYTHON_TOOL_MANIFEST_DESCRIPTION = "description"
PYTHON_TOOL_MANIFEST_SCHEMA = "schema"
PYTHON_TOOL_MANIFEST_SOURCE_FILE = "source_file"
PYTHON_TOOLS_AIR_COMMENT_PREFIX = "; __apxm_python_tools__ "

_g = globals()
for _spec in _ops.ALL_OPERATIONS:
    _g[f"OP_{_spec.op}"] = _spec.op
del _g, _spec, _ops
