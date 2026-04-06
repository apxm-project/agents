"""Graph constants for APXM Python frontend.

Re-exports all constants from apxm._generated.constants and apxm._generated.operations.
"""

from apxm._generated.constants import *  # noqa: F401, F403
from apxm._generated import operations

# LLM_OPS: frozenset of operation names that perform LLM reasoning
LLM_OPS: frozenset[str] = frozenset(
    spec.op for spec in [
        operations.ASK,
        operations.THINK,
        operations.REASON,
        operations.PLAN,
        operations.REFLECT,
        operations.VERIFY,
    ]
)

# Operation name constants from generated specs
OP_AGENT = operations.AGENT.op
OP_QMEM = operations.QMEM.op
OP_UMEM = operations.UMEM.op
OP_ASK = operations.ASK.op
OP_THINK = operations.THINK.op
OP_REASON = operations.REASON.op
OP_PLAN = operations.PLAN.op
OP_REFLECT = operations.REFLECT.op
OP_VERIFY = operations.VERIFY.op
OP_INV = operations.INV_TOOL.op
OP_EXC = operations.EXC.op
OP_PRINT = operations.PRINT.op
OP_JUMP = operations.JUMP.op
OP_BRANCH_ON_VALUE = operations.BRANCH_ON_VALUE.op
OP_LOOP_START = operations.LOOP_START.op
OP_LOOP_END = operations.LOOP_END.op
OP_RETURN = operations.RETURN.op
OP_SWITCH = operations.SWITCH.op
OP_FLOW_CALL = operations.FLOW_CALL.op
OP_MERGE = operations.MERGE.op
OP_FENCE = operations.FENCE.op
OP_WAIT_ALL = operations.WAIT_ALL.op
OP_TRY_CATCH = operations.TRY_CATCH.op
OP_ERR = operations.ERR.op
OP_COMMUNICATE = operations.COMMUNICATE.op
OP_UPDATE_GOAL = operations.UPDATE_GOAL.op
OP_GUARD = operations.GUARD.op
OP_CLAIM = operations.CLAIM.op
OP_PAUSE = operations.PAUSE.op
OP_RESUME = operations.RESUME.op
OP_DELEGATE = operations.DELEGATE.op
OP_NEGOTIATE = operations.NEGOTIATE.op
OP_NOP = operations.NOP.op
OP_IDENTITY = operations.IDENTITY.op
OP_SPAWN_AGENT = operations.SPAWN_AGENT.op
OP_REGISTER_CAPABILITY = operations.REGISTER_CAPABILITY.op
OP_AUTONOMOUS = operations.AUTONOMOUS.op
OP_CONST_STR = operations.CONST_STR.op
OP_YIELD = operations.YIELD.op
