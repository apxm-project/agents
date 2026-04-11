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

_g = globals()
for _spec in _ops.ALL_OPERATIONS:
    _g[f"OP_{_spec.op}"] = _spec.op
del _g, _spec, _ops
