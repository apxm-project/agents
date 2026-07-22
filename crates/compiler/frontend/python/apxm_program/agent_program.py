"""Typed Agent Program authoring on the canonical five-op FrontendGraph surface."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable, Iterable, Optional

from . import _FrontendGraphRecorder, canonical_air_json, lower, verify
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
    """Language-native recorder for one authored Agent Program."""

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
        self._builder = _FrontendGraphRecorder(source_language=source_language)
        self._builder.program(
            program_id=program_id,
            entrypoint=entrypoint,
            input_type_ref=input_type_ref,
            output_type_ref=output_type_ref,
            has_default_context=has_default_context,
            context_type_ref=context_type_ref,
        )

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

    def model_call(
        self, node_id: str, model_target_ref: str | None = None
    ) -> "AgentProgram":
        self._builder.model_call(node_id, model_target_ref=model_target_ref)
        if model_target_ref is not None:
            self._builder.model_requirement(model_target_ref)
        return self

    def capability_invoke(
        self, node_id: str, capability_ref: str | None = None
    ) -> "AgentProgram":
        self._builder.capability_invoke(node_id, capability_ref=capability_ref)
        if capability_ref is not None:
            self._builder.capability_requirement(capability_ref)
        return self

    def await_event(self, node_id: str, event_ref: str) -> "AgentProgram":
        self._builder.await_event(node_id, event_ref=event_ref)
        return self

    def loop(
        self,
        region_id: str,
        body: Callable[["AgentProgram"], object],
    ) -> "AgentProgram":
        self._builder.annotate_region(region_id, "structural_loop")
        return self._structured_region(region_id, "ais.loop", body)

    def branch(
        self,
        region_id: str,
        then_body: Callable[["AgentProgram"], object],
        else_body: Callable[["AgentProgram"], object] | None = None,
    ) -> "AgentProgram":
        def body(program: "AgentProgram") -> None:
            then_body(program)
            if else_body is not None:
                else_body(program)

        return self._structured_region(region_id, "branch", body)

    def switch(
        self,
        region_id: str,
        cases: Iterable[Callable[["AgentProgram"], object]],
    ) -> "AgentProgram":
        def body(program: "AgentProgram") -> None:
            for case in cases:
                case(program)

        return self._structured_region(region_id, "switch", body)

    def parallel(
        self,
        region_id: str,
        body: Callable[["AgentProgram"], object],
    ) -> "AgentProgram":
        return self._structured_region(region_id, "parallel_join", body)

    def try_catch(
        self,
        try_region_id: str,
        catch_region_id: str,
        try_body: Callable[["AgentProgram"], object],
        catch_body: Callable[["AgentProgram"], object],
    ) -> "AgentProgram":
        self._structured_region(try_region_id, "try", try_body)
        self._structured_region(catch_region_id, "catch", catch_body)
        return self

    def throw_region(self, region_id: str) -> "AgentProgram":
        self._builder.region(region_id, "throw")
        return self

    def return_region(self, region_id: str) -> "AgentProgram":
        self._builder.return_region(region_id)
        return self

    def yield_region(self, region_id: str) -> "AgentProgram":
        self._builder.yield_region(region_id)
        return self

    def source_span(
        self,
        node_id: str,
        source_file: str,
        line: int,
        semantic_annotation: str,
        start_column: int = 0,
        end_column: int | None = None,
    ) -> "AgentProgram":
        self._builder.node_span(
            node_id,
            source_file,
            line,
            semantic_annotation,
            start_column,
            end_column,
        )
        return self

    def context_flow(self, edge: ContextEdge) -> "AgentProgram":
        self._builder.context_edge(edge.from_node, edge.to_node, edge.context_type_ref)
        return self

    def annotate_region(self, region_id: str, annotation: str) -> "AgentProgram":
        self._builder.annotate_region(region_id, annotation)
        return self

    def _record_region(self, region_id: str, kind: str) -> None:
        self._builder.region(region_id, kind)

    def _enter_region(self, region_id: str) -> Optional[str]:
        return self._builder.enter_region(region_id)

    def _leave_region(self, previous: Optional[str]) -> None:
        self._builder.leave_region(previous)

    def _structured_region(
        self,
        region_id: str,
        kind: str,
        body: Callable[["AgentProgram"], object],
    ) -> "AgentProgram":
        self._builder.region(region_id, kind)
        previous = self._builder.enter_region(region_id)
        try:
            body(self)
        finally:
            self._builder.leave_region(previous)
        return self

    def _record_program_invoke(self, node_id: str, spec: ProgramInvokeSpec) -> None:
        self._builder.program_invoke(node_id, operands=spec.to_operands())

    def build_graph(self) -> dict[str, Any]:
        return self._builder.build()

    def verify(self) -> Optional[str]:
        return verify(self.build_graph())

    def lower(self) -> dict[str, Any]:
        return lower(self.build_graph())

    def canonical_air_json(self) -> str:
        return canonical_air_json(self.build_graph())
