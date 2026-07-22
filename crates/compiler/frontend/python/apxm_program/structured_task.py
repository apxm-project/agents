"""Structured task scope — parallel child composition with a typed join."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING

from .program_instance import ProgramInstanceRef, ProgramInvokeSpec, ProgramRef

if TYPE_CHECKING:
    from .agent_program import AgentProgram


@dataclass
class StructuredTaskScope:
    """Language-neutral structured task scope over public five-op APIs.

    Lowers to a parallel_join structural region whose exit joins every attached
    child invocation. There is no detach form.
    """

    program: "AgentProgram"
    region_id: str
    _invoke_node_ids: list[str] = field(default_factory=list)

    def __enter__(self) -> "StructuredTaskScope":
        self.program.builder.region(self.region_id, "parallel_join")
        return self

    def __exit__(self, *_exc: object) -> None:
        return None

    def invoke_program(
        self,
        node_id: str,
        receiver: ProgramRef | ProgramInstanceRef,
        *,
        input_payload: dict | None = None,
    ) -> "StructuredTaskScope":
        spec = ProgramInvokeSpec(receiver=receiver, input_payload=input_payload)
        self.program.builder.program_invoke(node_id, operands=spec.to_operands())
        self._invoke_node_ids.append(node_id)
        return self
