"""The Python frontend facts the shared conformance corpus cannot state.

Everything about authoring a program — what an Agent binds, what the
FrontendGraph says, what the canonical AIR lowers to, and what authoring is
rejected — is stated once in `contracts/vectors/apxm.frontend-conformance.json`
and run from the generated harness in `test_frontend_conformance.py`. A fact
that only exists in this language stays here, because a corpus vector projected
into both languages could not state it:

* A bare typed marker factory (``Model[object]``) is a Python subscript
  expression that is not yet a declaration. TypeScript has no such intermediate
  value — `Model<Input, Output>` is not a value at all — so the fact has no
  TypeScript projection.
* The generated runtime-evidence binding decodes facts. It is a generated
  contract binding rather than an authored Agent, so it has no source fixture to
  capture and no FrontendGraph to compare.
* A ``Skill`` stating both an entry and inline text is a rejection the corpus's
  ``declaration_rejections`` cannot express: that vocabulary states one
  reference per marker and calls the marker with it, so it can state the skill
  that names neither source — and does — but not the one that names two. The
  TypeScript frontend carries the same test for the same reason.
* A Hook whose target names nothing, and a Hook that names its target
  declaration rather than the selector for it. A Python Hook binds through the
  module globals its Agent resolves, so both are facts about a whole module, not
  about one authored program, and a corpus vector authors its programs inside
  one shared module. The TypeScript frontend carries the same tests for the same
  reason.
"""

from __future__ import annotations

import pytest

from apxm_program import Agent, Event, Hook, Model, Skill
from apxm_program._capture import CaptureError
from apxm_program._generated.diagnostics import (
    HOOK_DYNAMIC_REGISTRATION,
    HOOK_SCOPE_UNRESOLVED,
    HOOK_TARGET_UNRESOLVED,
    SKILL_ENTRY_PATH_NOT_CANONICAL,
    SKILL_SOURCE_AMBIGUOUS,
)
from apxm_program._generated.runtime_evidence import (
    LoopIterationCompletedFact,
    decode_fact,
)


def test_a_bare_typed_marker_factory_is_not_yet_a_declaration() -> None:
    bare_typed_factory = Event[object]
    assert not hasattr(bare_typed_factory, "wait")

    event = bare_typed_factory("event.session.input")
    assert event.target_ref == "event.session.input"


class _Payload:
    """A typed declaration this module's fixtures state an interface with."""


_StrayModel = Model[_Payload, _Payload]("stray.hook.model")


async def _stray_handler(agent) -> None:
    return None


def test_a_hook_target_no_declaration_names_is_refused() -> None:
    """A typo'd Hook target is refused rather than dropped.

    The Hook has to live in module globals for the capture to resolve it at all,
    and it is removed again afterwards: a Hook left in scope binds into every
    later capture in this module, which is what the generated corpus harness
    does for the same reason.
    """
    global _StrayHook
    _StrayHook = Hook.before(target="NoDeclarationNamesThis", scope="model")(
        _stray_handler
    )
    try:
        with pytest.raises(CaptureError, match=HOOK_TARGET_UNRESOLVED):

            @Agent(input=_Payload, output=_Payload)
            async def StrayHookTarget(agent, input):
                return await _StrayModel(input)

    finally:
        del _StrayHook


def test_a_hook_names_its_target_declaration_or_the_selector_for_it() -> None:
    """``target=_StrayModel`` and ``target="_StrayModel"`` are one statement.

    TypeScript has always taken the declaration; Python took only the selector,
    so the same Hook had to be written two ways. Both resolve to the same bound
    target here, which is what makes them one authoring shape rather than two.
    """
    global _StrayHook
    captured = []
    for target in (_StrayModel, "_StrayModel"):
        _StrayHook = Hook.before(target=target, scope="model")(_stray_handler)
        try:

            @Agent(input=_Payload, output=_Payload)
            async def HookTargetShape(agent, input):
                return await _StrayModel(input)

            captured.append(
                HookTargetShape.frontend_graph()["hook_bindings"][0]["target_selector"]
            )
        finally:
            del _StrayHook
    assert captured[0] == captured[1]


def test_a_hook_target_that_is_neither_declaration_nor_selector_is_refused() -> None:
    with pytest.raises(TypeError, match=HOOK_DYNAMIC_REGISTRATION):
        Hook.before(target=object(), scope="model")


def test_a_hook_scope_outside_the_vocabulary_is_refused() -> None:
    with pytest.raises(ValueError, match=HOOK_SCOPE_UNRESOLVED):
        Hook.before(target=_StrayModel, scope="capabilities")


def test_a_skill_states_one_instruction_source() -> None:
    with pytest.raises(ValueError, match=SKILL_SOURCE_AMBIGUOUS):
        Skill("review", entry="skills/review/SKILL.md", text="Review carefully.")


def test_a_file_carried_skill_names_the_path_its_id_resolves_to() -> None:
    with pytest.raises(ValueError, match=SKILL_ENTRY_PATH_NOT_CANONICAL):
        Skill("review", entry="prompts/review.md")


def test_generated_runtime_evidence_binding_is_closed() -> None:
    fact = decode_fact(
        {
            "fact_id": "loop.1",
            "event_sequence": 1,
            "fact_kind": "LoopIterationCompleted",
            "static_loop_id": "loop.main",
            "loop_occurrence_id": "occurrence.1",
            "iteration_index": 0,
            "program_invocation_id": "invocation.1",
            "causal_node_execution_ids": ["node-execution.1"],
        }
    )
    assert isinstance(fact, LoopIterationCompletedFact)

    try:
        decode_fact(
            {
                "fact_id": "bad.1",
                "event_sequence": 1,
                "fact_kind": "invented.fact",
            }
        )
    except ValueError:
        pass
    else:
        raise AssertionError("unknown fact kind must fail closed")
