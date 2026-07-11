"""Deterministic checks for the Python Gao reference profile."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys


PROFILE_ROOT = Path(__file__).resolve().parents[5] / "examples" / "python" / "gao"
PROFILE_PATH = PROFILE_ROOT / "gao_profile.py"
PACKAGE_PROFILE_PATH = PROFILE_ROOT.parent.parent / "agents" / "gao" / "python" / "gao_agent.py"


def load_gao():
    spec = importlib.util.spec_from_file_location("gao_reference_profile", PROFILE_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_package_gao():
    spec = importlib.util.spec_from_file_location("gao_agent_package_profile", PACKAGE_PROFILE_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_profile_loads_and_validates() -> None:
    gao = load_gao()
    profile = gao.load_profile()
    assert profile.flow_names == ["main", "conversation.turn"]
    assert profile.validate().valid


def test_live_turn_returns_citation_and_applyable_canvas() -> None:
    gao = load_gao()
    result = gao.run_live_turn("How does dependency-driven execution work?")

    assert "Gao, Patel, and St. John" in result.answer
    assert result.paper_result["matches"]
    assert result.canvas["format"] == "apxm_studio_workflow"
    assert [node["kind"] for node in result.canvas["nodes"]] == ["text", "llm", "output"]
    assert result.canvas["edges"] == [
        {"source": "question", "target": "answer", "kind": "data"},
        {"source": "answer", "target": "result", "kind": "data"},
    ]


def test_no_match_turn_does_not_fabricate_citation() -> None:
    gao = load_gao()
    result = gao.run_live_turn("What is the weather on Mars tomorrow?")

    assert result.paper_result["matches"] == []
    assert result.answer == "No source found in the local paper corpus."
    assert "Source:" not in result.answer


def test_provider_policy_fails_closed_without_connection() -> None:
    gao = load_gao()

    assert gao.provider_capabilities(None) == ()
    assert gao.provider_capabilities(gao.ProviderConnection(reference="")) == ()
    assert gao.SEARCH_WEB_CAPABILITY not in gao.capability_groups()
    assert gao.SEARCH_WEB_CAPABILITY in gao.provider_capabilities(
        gao.ProviderConnection(reference="connection/search")
    )
    assert "web" in gao.capability_groups(gao.ProviderConnection(reference="connection/search"))


def test_live_turn_graph_uses_python_capability_composition() -> None:
    gao = load_gao()
    graph = gao.gao_turn._graph
    capability_names = {
        node.attributes.get("capability")
        for node in graph.nodes
        if node.op == "INV_CAP"
    }

    assert {"search_papers", "plan_workflow"} <= capability_names
    assert 'TAVILY_API_KEY' not in gao.gao_turn.to_air()
    assert '"web"' not in gao.gao_turn.to_air()


def test_agent_package_exposes_the_same_live_turn_and_provider_policy() -> None:
    gao = load_package_gao()
    result = gao.run_live_turn("How does dependency-driven execution work?")

    assert "Gao, Patel, and St. John" in result["answer"]
    assert result["canvas"]["format"] == "apxm_studio_workflow"
    assert gao.provider_capabilities(gao.ProviderConnection(reference=None)) == ()
    connected = gao.build_agent(gao.ProviderConnection(reference="connection/search"))
    assert '"web"' in connected.to_air()
