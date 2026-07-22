"""Conversational Agent Program composition on public five-op authoring APIs."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

from .agent_program import AgentProgram, ContextEdge, HookBinding, ImportedProgram


@dataclass(frozen=True, slots=True)
class TurnSpec:
    """One authored conversational Turn inside a structural loop region."""

    model_node_id: str = "node.turn.model"
    capability_node_id: str = "node.turn.tool"
    await_node_id: str = "node.turn.await"
    model_target_ref: str = "model.default"
    capability_ref: Optional[str] = None
    event_ref: str = "event.turn.input"


@dataclass(frozen=True, slots=True)
class SpecialistComposition:
    """Typed specialist Program Instance composition inside a Turn."""

    import_ref: ImportedProgram
    new_node_id: str = "node.specialist.new"
    invoke_node_id: str = "node.specialist.invoke"


class ConversationalAgent(AgentProgram):
    """Compose a conversational loop from public Agent Program constructs only.

    The loop is a structural region annotated as a conversational loop. Each Turn
    authors explicit `model.call`, optional `capability.invoke`, optional
    `program.new`/`program.invoke`, and `await.event` park/resume semantics. No
    retired operation, hidden runtime loop, or compiler branch is introduced.
    """

    LOOP_REGION_SUFFIX = "loop.turn"
    RETURN_REGION_ID = "region.return"

    def __init__(
        self,
        *,
        program_id: str,
        input_type_ref: str,
        output_type_ref: str,
        context_type_ref: str,
        entrypoint: str = "run",
        source_language: str = "python",
    ) -> None:
        super().__init__(
            program_id=program_id,
            entrypoint=entrypoint,
            input_type_ref=input_type_ref,
            output_type_ref=output_type_ref,
            has_default_context=True,
            context_type_ref=context_type_ref,
            source_language=source_language,
        )
        self._loop_region_id = f"region.{self.LOOP_REGION_SUFFIX}"
        self._specialist: Optional[SpecialistComposition] = None
        self._source_file = f"{program_id}.{'ts' if source_language == 'typescript' else 'py'}"

    @property
    def loop_region_id(self) -> str:
        return self._loop_region_id

    def with_specialist(self, composition: SpecialistComposition) -> "ConversationalAgent":
        self._specialist = composition
        return self

    def define_turn(
        self,
        turn: TurnSpec | None = None,
        *,
        hooks: tuple[HookBinding, ...] = (),
    ) -> "ConversationalAgent":
        spec = turn or TurnSpec()
        builder = self.builder
        if self._specialist is not None:
            self.import_program(self._specialist.import_ref)
        builder.model_call(spec.model_node_id, model_target_ref=spec.model_target_ref)
        if spec.capability_ref is not None:
            builder.capability_invoke(spec.capability_node_id, capability_ref=spec.capability_ref)
        if self._specialist is not None:
            builder.program_new(self._specialist.new_node_id)
            builder.program_invoke(self._specialist.invoke_node_id)
        builder.await_event(spec.await_node_id, event_ref=spec.event_ref)
        builder.region(self._loop_region_id, "loop")
        builder.return_region(self.RETURN_REGION_ID)
        context_sink = (
            spec.capability_node_id if spec.capability_ref is not None else spec.await_node_id
        )
        self.context_flow(
            ContextEdge(spec.model_node_id, context_sink, self.context_type_ref or "")
        )
        for hook in hooks:
            self.bind_hook(hook)
        builder.model_requirement(spec.model_target_ref)
        if spec.capability_ref is not None:
            builder.capability_requirement(spec.capability_ref)
        source_nodes = [
            (spec.model_node_id, "model.call"),
            *(([(spec.capability_node_id, "capability.invoke")]) if spec.capability_ref is not None else []),
            *(([(self._specialist.new_node_id, "program.new"), (self._specialist.invoke_node_id, "program.invoke")]) if self._specialist is not None else []),
            (spec.await_node_id, "await.event"),
        ]
        for line, (node_id, annotation) in enumerate(source_nodes, start=1):
            builder.node_span(node_id, self._source_file, line, annotation)
        return self.annotate_region(self._loop_region_id, "conversational_loop")
