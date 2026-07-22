"""Conversational example parity across installed generic frontends."""

from __future__ import annotations

import json
import importlib.util
import subprocess
from pathlib import Path

from apxm_program import AgentProgram

EXAMPLE_DIR = Path(__file__).resolve().parents[1]
PYTHON_SOURCE = EXAMPLE_DIR / "python" / "agent.py"

spec = importlib.util.spec_from_file_location("conversational_example", PYTHON_SOURCE)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
build_conversational = module.build_conversational


def node_value(expression: str) -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            (
                "import { buildConversational } from './dist/index.js';"
                f"console.log(JSON.stringify({expression}));"
            ),
        ],
        cwd=EXAMPLE_DIR,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def node_structural_graph() -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            """
import { AgentProgram } from "@apxm/frontend";
const program = new AgentProgram({
  program_id: "StructuralParity",
  input_type_ref: "Input",
  output_type_ref: "Output",
});
const empty = () => undefined;
program.branch("region.branch", empty, empty);
program.switch("region.switch", [empty, empty]);
program.loop("region.loop", empty);
program.parallel("region.parallel", empty);
program.tryCatch("region.try", "region.catch", empty, empty);
program.throwRegion("region.throw");
program.returnRegion("region.return");
program.yieldRegion("region.yield");
console.log(JSON.stringify(program.buildGraph()));
""",
        ],
        cwd=EXAMPLE_DIR,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def node_nested_loop_graph() -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            """
import { AgentProgram } from "@apxm/frontend";
const program = new AgentProgram({
  program_id: "NestedParity",
  input_type_ref: "Input",
  output_type_ref: "Output",
});
program.loop("loop.outer", (outer) => {
  outer.modelCall("node.outer.before", "model.default");
  outer.loop("loop.inner", (inner) => inner.capabilityInvoke("node.inner", "cap.inner"));
  outer.modelCall("node.outer.after", "model.default");
});
program.loop("loop.sibling", (sibling) => sibling.capabilityInvoke("node.sibling", "cap.sibling"));
console.log(JSON.stringify(program.buildGraph()));
""",
        ],
        cwd=EXAMPLE_DIR,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def normalize_source(value: dict) -> dict:
    normalized = json.loads(json.dumps(value))
    normalized["source_language"] = "frontend"
    source_map = normalized["source_map"]
    source_map["source_language"] = "frontend"
    for span in source_map["node_spans"]:
        span["source_file"] = "frontend"
        span["span"] = {
            "start_line": 0,
            "start_column": 0,
            "end_line": 0,
            "end_column": 0,
        }
    return normalized


def test_python_and_typescript_record_equivalent_frontend_graphs() -> None:
    python_graph = build_conversational().build_graph()
    typescript_graph = node_value("buildConversational().buildGraph()")
    assert normalize_source(python_graph) == normalize_source(typescript_graph)


def test_python_and_typescript_lower_to_equivalent_air_structure() -> None:
    python_air = build_conversational().lower()
    typescript_air = node_value("buildConversational().lowerGraph()")
    assert normalize_source(python_air) == normalize_source(typescript_air)
    assert "ais.loop" in json.dumps(python_air, sort_keys=True)


def test_python_and_typescript_record_equivalent_structural_kinds() -> None:
    python = AgentProgram(
        program_id="StructuralParity",
        input_type_ref="Input",
        output_type_ref="Output",
    )

    def empty(_body: AgentProgram) -> None:
        return None

    python.branch("region.branch", empty, empty)
    python.switch("region.switch", (empty, empty))
    python.loop("region.loop", empty)
    python.parallel("region.parallel", empty)
    python.try_catch("region.try", "region.catch", empty, empty)
    python.throw_region("region.throw")
    python.return_region("region.return")
    python.yield_region("region.yield")

    assert normalize_source(python.build_graph()) == normalize_source(node_structural_graph())


def test_generic_examples_preserve_nested_and_sibling_loop_containment() -> None:
    python = AgentProgram(
        program_id="NestedParity",
        input_type_ref="Input",
        output_type_ref="Output",
    )

    def outer(body: AgentProgram) -> None:
        body.model_call("node.outer.before", "model.default")
        body.loop(
            "loop.inner",
            lambda inner: inner.capability_invoke("node.inner", "cap.inner"),
        )
        body.model_call("node.outer.after", "model.default")

    python.loop("loop.outer", outer)
    python.loop(
        "loop.sibling",
        lambda sibling: sibling.capability_invoke("node.sibling", "cap.sibling"),
    )

    python_graph = python.build_graph()
    assert normalize_source(python_graph) == normalize_source(node_nested_loop_graph())
    regions = {
        region["region_id"]: region
        for region in python_graph["structural_regions"]
    }
    assert regions["loop.inner"]["parent_region_id"] == "loop.outer"
    assert regions["loop.sibling"]["parent_region_id"] == "region.NestedParity.body"
    assert "conversational_loop" not in json.dumps(python_graph, sort_keys=True)


def test_conversational_source_spans_point_to_authored_calls() -> None:
    cases = [
        (
            build_conversational().build_graph(),
            EXAMPLE_DIR / "python" / "agent.py",
            {
                "node.model": '.model_call("node.model"',
                "node.capability": '.capability_invoke("node.capability"',
                "node.await": '.await_event("node.await"',
            },
        ),
        (
            node_value("buildConversational().buildGraph()"),
            EXAMPLE_DIR / "src" / "conversational-agent.ts",
            {
                "node.model": '.modelCall("node.model"',
                "node.capability": '.capabilityInvoke("node.capability"',
                "node.await": '.awaitEvent("node.await"',
            },
        ),
    ]
    for graph, source_path, expected in cases:
        lines = source_path.read_text().splitlines()
        spans = {
            span["node_id"]: span
            for span in graph["source_map"]["node_spans"]
        }
        for node_id, authored_call in expected.items():
            span = spans[node_id]["span"]
            line = lines[span["start_line"] - 1]
            assert line[span["start_column"] : span["end_column"]] == authored_call
