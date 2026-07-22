"""ConversationalAgent public API parity with the shared Gao example graph."""

from __future__ import annotations

import apxm_program
from apxm_program.gao import build_gao, gao_conversational_graph


def gao_via_conversational_agent() -> dict:
    return build_gao().build_graph()


def test_conversational_agent_matches_gao_example_graph():
    assert gao_via_conversational_agent() == gao_conversational_graph()


def test_conversational_agent_lowers_to_golden_air():
    graph = gao_via_conversational_agent()
    assert apxm_program.verify(graph) is None
    from apxm_program.example import gao_conversational_graph as golden_graph

    assert apxm_program.canonical_air_json(graph) == apxm_program.canonical_air_json(
        golden_graph()
    )


def test_public_conversational_agent_builds_a_complete_executable_artifact():
    graph = gao_via_conversational_agent()
    artifact = apxm_program.compile_artifact(graph)

    assert artifact["schema_version"] == "apxm.executable-artifact.v1"
    assert artifact["air"] == apxm_program.lower(graph)
    assert artifact["entrypoints"][0]["program_id"] == "Gao"
