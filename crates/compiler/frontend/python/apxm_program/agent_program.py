"""Typed Agent Program authoring on the canonical five-op FrontendGraph surface."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional

from . import GraphBuilder, canonical_air_json, lower, verify
from .program_instance import ProgramInstanceRef, ProgramInvokeSpec, ProgramNewSpec, ProgramRef


@dataclass(frozen=True, slots=True)
class HookBinding:
    """Static Hook binding recorded on the FrontendGraph."""

    hook_id: str
    scope: str
    phase: str
    target_selector: str
    declaration_order: int
    handler_ref: str
    handler_digest: str
    input_type_ref: str
    output_type_ref: str
    return_mode: str

    def to_graph_fields(self) -> dict[str, Any]:
        return {
            "hook_id": self.hook_id,
            "scope": self.scope,
            "phase": self.phase,
            "target_selector": self.target_selector,
            "declaration_order": self.declaration_order,
            "handler_ref": self.handler_ref,
            "handler_digest": self.handler_digest,
            "input_type_ref": self.input_type_ref,
            "output_type_ref": self.output_type_ref,
            "return_mode": self.return_mode,
        }


@dataclass(frozen=True, slots=True)
class ContextEdge:
    """Explicit typed Context flow between two authored nodes."""

    from_node: str
    to_node: str
    context_type_ref: str


@dataclass(frozen=True, slots=True)
class ImportedProgram:
    """Digest-pinned import of an externally compiled specialist program."""

    program_ref: str
    artifact_digest: str
    entrypoint: str
    target_agent_identity_requirement: str


class AgentProgram:
    """Language-native wrapper over GraphBuilder for one authored Agent Program."""

    def __init__(
        self,
        *,
        program_id: str,
        entrypoint: str = "run",
        input_type_ref: str,
        output_type_ref: str,
        has_default_context: bool = True,
        context_type_ref: Optional[str] = None,
        source_language: str = "python",
    ) -> None:
        self.program_id = program_id
        self.context_type_ref = context_type_ref
        self._builder = GraphBuilder(source_language=source_language)
        self._builder.program(
            program_id=program_id,
            entrypoint=entrypoint,
            input_type_ref=input_type_ref,
            output_type_ref=output_type_ref,
            has_default_context=has_default_context,
            context_type_ref=context_type_ref,
        )

    @property
    def builder(self) -> GraphBuilder:
        return self._builder

    def import_program(self, imported: ImportedProgram) -> "AgentProgram":
        self._builder.import_program(
            program_ref=imported.program_ref,
            artifact_digest=imported.artifact_digest,
            entrypoint=imported.entrypoint,
            target_agent_identity_requirement=imported.target_agent_identity_requirement,
        )
        return self

    def bind_hook(self, hook: HookBinding) -> "AgentProgram":
        self._builder.hook(**hook.to_graph_fields())
        return self

    def program_new(
        self,
        node_id: str,
        *,
        spec: ProgramNewSpec | None = None,
        program_ref: str | None = None,
    ) -> tuple["AgentProgram", ProgramInstanceRef]:
        operands = spec.to_operands() if spec is not None else None
        if operands is None and program_ref is not None:
            operands = ProgramNewSpec(program_ref=program_ref).to_operands()
        self._builder.program_new(node_id, operands=operands)
        ref = program_ref or (spec.program_ref if spec is not None else node_id)
        return self, ProgramInstanceRef(instance_node_id=node_id, program_ref=ref)

    def program_invoke(
        self,
        node_id: str,
        *,
        receiver: ProgramRef | ProgramInstanceRef,
        input_payload: dict | None = None,
    ) -> "AgentProgram":
        spec = ProgramInvokeSpec(receiver=receiver, input_payload=input_payload)
        self._builder.program_invoke(node_id, operands=spec.to_operands())
        return self

    def structured_task(self, region_id: str) -> "StructuredTaskScope":
        from .structured_task import StructuredTaskScope

        return StructuredTaskScope(self, region_id)

    def context_flow(self, edge: ContextEdge) -> "AgentProgram":
        self._builder.context_edge(edge.from_node, edge.to_node, edge.context_type_ref)
        return self

    def annotate_region(self, region_id: str, annotation: str) -> "AgentProgram":
        self._builder.annotate_region(region_id, annotation)
        return self

    def build_graph(self) -> dict[str, Any]:
        return self._builder.build()

    def verify(self) -> Optional[str]:
        return verify(self.build_graph())

    def lower(self) -> dict[str, Any]:
        return lower(self.build_graph())

    def canonical_air_json(self) -> str:
        return canonical_air_json(self.build_graph())
