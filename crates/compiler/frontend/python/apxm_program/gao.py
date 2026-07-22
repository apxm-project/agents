"""Gao — a named specialization of the public ConversationalAgent construct."""

from __future__ import annotations

from typing import Any

from .conversational import ConversationalAgent, SpecialistComposition, TurnSpec
from .hook import Hook
from .agent_program import ImportedProgram

DIGEST_SPECIALIST = "sha256:" + "c" * 64
DIGEST_HOOK_BEFORE = "sha256:" + "d" * 64
DIGEST_HOOK_AFTER = "sha256:" + "e" * 64


def build_gao() -> ConversationalAgent:
    """Author Gao through public ConversationalAgent APIs only."""
    return (
        ConversationalAgent(
            program_id="Gao",
            input_type_ref="GaoInput",
            output_type_ref="GaoOutput",
            context_type_ref="GaoContext",
        )
        .with_specialist(
            SpecialistComposition(
                import_ref=ImportedProgram(
                    program_ref="Specialist",
                    artifact_digest=DIGEST_SPECIALIST,
                    entrypoint="run",
                    target_agent_identity_requirement="specialist-identity",
                )
            )
        )
        .define_turn(
            TurnSpec(capability_ref="cap.search"),
            hooks=(
                Hook.before_loop(
                    hook_id="hook.before.turn",
                    target_selector="region.loop.turn",
                    handler_ref="hooks.before_turn",
                    handler_digest=DIGEST_HOOK_BEFORE,
                    context_type_ref="GaoContext",
                ).to_binding(),
                Hook.after_model(
                    hook_id="hook.after.model",
                    target_selector="node.turn.model",
                    handler_ref="hooks.after_model",
                    handler_digest=DIGEST_HOOK_AFTER,
                    result_type_ref="ModelResult",
                    declaration_order=1,
                ).to_binding(),
            ),
        )
    )


def gao_conversational_graph() -> dict[str, Any]:
    return build_gao().build_graph()
