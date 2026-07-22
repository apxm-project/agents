"""Generated five-operation wire names — SSOT re-export for apxm_program."""

from __future__ import annotations

from typing import Final

OP_MODEL_CALL: Final = "model.call"
OP_CAPABILITY_INVOKE: Final = "capability.invoke"
OP_PROGRAM_NEW: Final = "program.new"
OP_PROGRAM_INVOKE: Final = "program.invoke"
OP_AWAIT_EVENT: Final = "await.event"

FIVE_OPS: Final[frozenset[str]] = frozenset(
    {
        OP_MODEL_CALL,
        OP_CAPABILITY_INVOKE,
        OP_PROGRAM_NEW,
        OP_PROGRAM_INVOKE,
        OP_AWAIT_EVENT,
    }
)
