"""Frontend-graph DTO parity tests.

Python does not format AIR text itself: `ApxmGraph.to_air()` /
`emit_multi_flow_module()` subprocess to `<apxm> emit-air`, which
deserializes the shared `FrontendGraph` DTO and calls the single Rust
printer (`AirModule::to_air()` / `AirProgram::to_air()`). These tests prove
that delegation actually produces byte-identical output to the canonical
printer for the same fixtures the Rust `frontend_air` tests and the
TypeScript `emit-air.test.ts` vitest suite check
(`crates/tools/cli/tests/fixtures/frontend_graph_parity/`), not just that
Python calls *some* subprocess.

`APXM_BIN`-gated, skip-if-absent (matches
`typescript/test/emit-air.test.ts:6-13`'s pattern): these tests need a real
built `apxm` binary to shell out to, which is not available in every
environment (e.g. a node-only or docs-only CI job). Set `APXM_BIN` to the
built binary to run them.
"""

from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from apxm.ir import (
    AirEmissionError,
    AirEmitterCommand,
    ApxmGraph,
    emit_multi_flow_module,
)

_FIXTURES_DIR = (
    Path(__file__).resolve().parents[4] / "tools" / "cli" / "tests" / "fixtures" / "frontend_graph_parity"
)

_APXM_BIN = os.environ.get("APXM_BIN")

requires_apxm_bin = pytest.mark.skipif(
    not _APXM_BIN,
    reason="APXM_BIN not set; skipping live `apxm emit-air` parity test",
)


def _load_fixture(name: str) -> dict:
    return json.loads((_FIXTURES_DIR / name).read_text())


def _load_golden(name: str) -> str:
    return (_FIXTURES_DIR / name).read_text()


def _graph_from_fixture(name: str) -> ApxmGraph:
    """Build a live `ApxmGraph` from a shared fixture via the real Python API.

    `ApxmGraph.from_dict` is the same entry point the Python frontend uses
    to round-trip its own `to_dict()` output; sourcing the dict from the
    on-disk fixture (rather than hand-duplicating node/edge construction)
    guarantees this test exercises the identical logical graph the Rust
    `frontend_air` tests and the TypeScript vitest suite compare against.
    """
    return ApxmGraph.from_dict(_load_fixture(name))


@requires_apxm_bin
def test_ask_flow_matches_golden_air():
    graph = _graph_from_fixture("ask_flow.json")
    air = graph.to_air()
    golden = _load_golden("ask_flow.golden.air")
    assert air == golden


@requires_apxm_bin
def test_parametrized_flow_matches_golden_air():
    graph = _graph_from_fixture("parametrized_flow.json")
    air = graph.to_air()
    golden = _load_golden("parametrized_flow.golden.air")
    assert air == golden


@requires_apxm_bin
def test_profiled_agent_flow_matches_golden_air():
    graph = _graph_from_fixture("profiled_agent_flow.json")
    air = graph.to_air()
    golden = _load_golden("profiled_agent_flow.golden.air")
    assert air == golden


@requires_apxm_bin
def test_multi_flow_conversational_matches_golden_air():
    """Cross-plane: Python's `emit_multi_flow_module` (`ir.py:396-398`) must
    match the same `AirProgram::to_air()` golden output the Rust
    `frontend_air` tests and TypeScript's `emitMultiFlowModule` also match
    (the shared multi-flow fixture)."""
    graphs = [ApxmGraph.from_dict(g) for g in _load_fixture("multi_flow_conversational.json")]
    air = emit_multi_flow_module(graphs)
    golden = _load_golden("multi_flow_conversational.golden.air")
    assert air == golden


def test_air_emitter_command_raises_air_emission_error_when_apxm_cannot_be_resolved():
    """Recovery: a failing/missing `apxm` binary fails loud with
    `AirEmissionError` naming the command and stderr/OS error detail — never
    silently returning empty or partial AIR (`ir.py:20-22,55-59`)."""
    bogus_binary = "/nonexistent/path/apxm-does-not-exist-w3-1"
    command = AirEmitterCommand(argv=(bogus_binary, "emit-air"))

    with pytest.raises(AirEmissionError) as excinfo:
        command.emit({"name": "unreachable", "nodes": [], "edges": [], "parameters": [], "metadata": {}})

    message = str(excinfo.value)
    assert bogus_binary in message
    assert "could not be started" in message
