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
"""

from __future__ import annotations

import pytest

from apxm_program import Event, Skill
from apxm_program._generated.diagnostics import (
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
