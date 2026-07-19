"""Python authoring frontend: record a FrontendGraph and lower it to canonical
AIR through the in-process PyO3 bridge (no CLI subprocess, no network)."""

from __future__ import annotations

import json
from pathlib import Path

import apxm_program
from apxm_program.example import external_agent_graph, specialist_graph

PARITY_DIR = Path(__file__).resolve().parents[4] / "compiler" / "frontend" / "native" / "parity"
GOLDEN_GRAPH = PARITY_DIR / "frontend-graph.example.json"
GOLDEN_AIR = PARITY_DIR / "air.expected.json"
GOLDEN_ACP_AIR = PARITY_DIR / "air.external-agent.expected.json"


def test_authored_graph_matches_golden_input():
    assert specialist_graph() == json.loads(GOLDEN_GRAPH.read_text())


def test_lower_matches_golden_air():
    air = apxm_program.canonical_air_json(specialist_graph())
    assert air == GOLDEN_AIR.read_text().strip()


def test_lowering_is_deterministic():
    graph = specialist_graph()
    assert apxm_program.canonical_air_json(graph) == apxm_program.canonical_air_json(graph)


def test_verify_accepts_valid_graph():
    assert apxm_program.verify(specialist_graph()) is None


def test_verify_rejects_unknown_operation():
    graph = specialist_graph()
    graph["semantic_operations"].append({"node_id": "node.bad", "op": "tool.loop"})
    assert apxm_program.verify(graph) is not None


def test_external_agent_capability_lowers_only_to_capability_invoke():
    air = apxm_program.lower(external_agent_graph())
    ops = [op["op"] for op in air["semantic_operations"]]
    assert ops == ["capability.invoke"]
    assert "model.call" not in ops


def test_external_agent_air_matches_golden():
    assert apxm_program.canonical_air_json(external_agent_graph()) == GOLDEN_ACP_AIR.read_text().strip()


def test_lower_rejects_unknown_operation():
    graph = specialist_graph()
    graph["semantic_operations"].append({"node_id": "node.bad", "op": "tool.loop"})
    try:
        apxm_program.lower(graph)
    except ValueError:
        return
    raise AssertionError("lowering an unknown operation must fail closed")
