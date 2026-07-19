"""The shared authoring example used to prove Python/TypeScript parity.

The TypeScript frontend authors the identical program, so both lower to
byte-identical canonical AIR.
"""

from __future__ import annotations

from typing import Any

from . import GraphBuilder

DIGEST_SUMMARIZER = "sha256:" + "a" * 64
DIGEST_HOOK = "sha256:" + "b" * 64


def specialist_graph() -> dict[str, Any]:
    builder = GraphBuilder(source_language="python")
    builder.program(
        program_id="Specialist",
        entrypoint="run",
        input_type_ref="SpecialistInput",
        output_type_ref="SpecialistOutput",
        has_default_context=True,
        context_type_ref="SpecialistContext",
    )
    builder.import_program(
        program_ref="Summarizer",
        artifact_digest=DIGEST_SUMMARIZER,
        entrypoint="run",
        target_agent_identity_requirement="summarizer-identity",
    )
    builder.model_call("node.model.1", model_target_ref="model.default")
    builder.capability_invoke("node.cap.1", capability_ref="cap.search")
    builder.program_new("node.new.1")
    builder.program_invoke("node.invoke.1")
    builder.await_event("node.await.1")
    builder.region("region.loop.1", "loop")
    builder.region("region.return.1", "return")
    builder.context_edge("node.model.1", "node.cap.1", "SpecialistContext")
    builder.hook(
        hook_id="hook.before.model",
        scope="model",
        phase="before",
        target_selector="node.model.1",
        declaration_order=0,
        handler_ref="hooks.before_model",
        handler_digest=DIGEST_HOOK,
        input_type_ref="ModelContext",
        output_type_ref="ModelContext",
        return_mode="observe",
    )
    builder.capability_requirement("cap.search")
    builder.model_requirement("model.default")
    return builder.build()


def external_agent_graph() -> dict[str, Any]:
    """A program that delegates to an External Agent over ACP.

    The peer runs its own private model/tool loop, but the program authors a
    single External Agent capability, so the AIR carries only one
    `capability.invoke` and no `model.call`.
    """
    builder = GraphBuilder(source_language="python")
    builder.program(
        program_id="Delegator",
        entrypoint="run",
        input_type_ref="DelegatorInput",
        output_type_ref="DelegatorOutput",
        has_default_context=True,
        context_type_ref="DelegatorContext",
    )
    builder.external_agent_capability(
        "node.acp.1",
        profile_ref="acp:claude-code",
        session_ref="session.acp.1",
    )
    builder.region("region.return.1", "return")
    builder.capability_requirement("external-agent:acp:claude-code")
    return builder.build()
