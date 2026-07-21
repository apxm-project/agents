"""Python canonical FrontendGraph and AIR parity fixtures."""

from __future__ import annotations

import json
from pathlib import Path

import apxm_program
from apxm_program.example import (
    external_agent_graph,
    gao_conversational_graph,
    session_agent_graph,
    specialist_graph,
)

_PARITY_DIR = Path(__file__).resolve().parents[2] / "native" / "parity"
_FIVE_OPS = {"model.call", "capability.invoke", "program.new", "program.invoke", "await.event"}


def _load_json(name: str) -> dict:
    return json.loads((_PARITY_DIR / name).read_text())


def _load_text(name: str) -> str:
    return (_PARITY_DIR / name).read_text().strip()


def test_python_authored_frontend_graph_matches_canonical_fixture():
    assert specialist_graph() == _load_json("frontend-graph.example.json")


def test_python_lowering_matches_canonical_air_fixture():
    assert apxm_program.canonical_air_json(specialist_graph()) == _load_text("air.expected.json")


def test_python_lowering_is_deterministic():
    graph = specialist_graph()
    assert apxm_program.canonical_air_json(graph) == apxm_program.canonical_air_json(graph)


def test_python_verify_rejects_retired_operation_names():
    graph = specialist_graph()
    graph["semantic_operations"].append({"node_id": "node.bad", "op": "ASK"})
    assert apxm_program.verify(graph) is not None


def test_external_agent_lowers_to_capability_invoke_only():
    air = apxm_program.lower(external_agent_graph())
    assert [op["op"] for op in air["semantic_operations"]] == ["capability.invoke"]


def test_external_agent_air_matches_canonical_fixture():
    assert apxm_program.canonical_air_json(external_agent_graph()) == _load_text(
        "air.external-agent.expected.json"
    )


def test_gao_conversational_agent_uses_only_canonical_semantic_ops():
    air = apxm_program.lower(gao_conversational_graph())
    for op in air["semantic_operations"]:
        assert op["op"] in _FIVE_OPS, op["op"]


def test_gao_air_matches_canonical_fixture():
    assert apxm_program.canonical_air_json(gao_conversational_graph()) == _load_text(
        "air.gao.expected.json"
    )


def test_session_agent_lowers_to_canonical_model_and_event_ops():
    air = apxm_program.lower(session_agent_graph())
    ops = [op["op"] for op in air["semantic_operations"]]
    assert "model.call" in ops
    assert "await.event" in ops
    for op in ops:
        assert op in _FIVE_OPS, op
