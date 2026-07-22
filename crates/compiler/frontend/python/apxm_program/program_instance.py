"""Typed program composition handles for program.new and program.invoke."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional


@dataclass(frozen=True, slots=True)
class ProgramRef:
    """One-shot receiver for program.invoke against an imported program."""

    program_ref: str


@dataclass(frozen=True, slots=True)
class ProgramInstanceRef:
    """Opaque typed handle produced by program.new for stateful program.invoke."""

    instance_node_id: str
    program_ref: str

    def receiver_operand(self) -> dict[str, str]:
        return {"program_instance_ref": self.instance_node_id}


@dataclass(frozen=True, slots=True)
class ProgramNewSpec:
    """Typed operands recorded on a program.new semantic operation."""

    program_ref: str
    initial_context: Optional[dict[str, Any]] = None

    def to_operands(self) -> dict[str, Any]:
        operands: dict[str, Any] = {"program_ref": self.program_ref}
        if self.initial_context is not None:
            operands["initial_context"] = self.initial_context
        return operands


@dataclass(frozen=True, slots=True)
class ProgramInvokeSpec:
    """Typed operands recorded on a program.invoke semantic operation."""

    receiver: ProgramRef | ProgramInstanceRef
    input_payload: Optional[dict[str, Any]] = None

    def to_operands(self) -> dict[str, Any]:
        if isinstance(self.receiver, ProgramInstanceRef):
            receiver = self.receiver.receiver_operand()
        else:
            receiver = {"program_ref": self.receiver.program_ref}
        operands: dict[str, Any] = {"receiver": receiver}
        if self.input_payload is not None:
            operands["input"] = self.input_payload
        return operands
