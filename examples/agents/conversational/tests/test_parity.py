"""The Python and TypeScript conversational examples agree after canonicalization."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

EXAMPLE_DIR = Path(__file__).resolve().parents[1]
FRONTEND_PACKAGE_DIR = EXAMPLE_DIR.parents[2] / ".apxm" / "frontend-example-python"


def _python_environment() -> dict[str, str]:
    """Keep the packed frontend importable after the subprocess changes cwd."""
    environment = os.environ.copy()
    inherited = environment.get("PYTHONPATH", "")
    environment["PYTHONPATH"] = os.pathsep.join(
        value for value in (str(FRONTEND_PACKAGE_DIR), inherited) if value
    )
    return environment


def python_air() -> dict:
    result = subprocess.run(
        ["python", "python/agent.py", "--air"],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def python_graph() -> dict:
    result = subprocess.run(
        ["python", "python/agent.py"],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def python_diagnostics() -> object:
    result = subprocess.run(
        ["python", "python/agent.py", "--diagnostics"],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def typescript_air() -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildConversational } from './dist/index.js';"
            "console.log(buildConversational().canonicalAir());",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def typescript_graph() -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildConversational } from './dist/index.js';"
            "console.log(JSON.stringify(buildConversational().frontendGraph()));",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def typescript_diagnostics() -> object:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildConversational } from './dist/index.js';"
            "console.log(JSON.stringify(buildConversational().diagnostics()));",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def _python_parity(mode: str = "") -> object:
    command = ["python", "python/parity.py"]
    if mode:
        command.append(mode)
    result = subprocess.run(
        command,
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def _typescript_parity(method: str) -> object:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildParityCorpus } from './dist/parity-agent.js';"
            f"const value = buildParityCorpus().{method}();"
            "console.log(typeof value === 'string' ? value : JSON.stringify(value));",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def _canonical_shape(air: dict) -> dict:
    """Project the semantics both languages must share, ignoring source spans."""
    return {
        "semantic_operations": [
            {
                "op": op["op"],
                "operand_slots": sorted(o["slot"] for o in op.get("operands", [])),
            }
            for op in air["semantic_operations"]
        ],
        "structural_ir": [node["kind"] for node in air["structural_ir"]],
    }


def _frontend_intent(graph: dict) -> dict:
    """Project typed FrontendGraph intent without language-specific spans or ids."""
    declarations = [
        {
            key: declaration.get(key)
            for key in (
                "decl_kind",
                "input_type_ref",
                "output_type_ref",
                "target_ref",
                "context_default_present",
            )
            if key in declaration
        }
        for declaration in graph["declarations"]
    ]
    return {
        "declarations": sorted(declarations, key=lambda declaration: json.dumps(declaration, sort_keys=True)),
        "call_intents": [call["intent_kind"] for call in graph["call_intents"]],
        "control_intents": [control["control_kind"] for control in graph["control_intents"]],
        "capability_requirements": graph["capability_requirements"],
        "model_requirements": graph["model_requirements"],
        "context_flow": [edge["context_type_ref"] for edge in graph["context_flow"]],
        "imported_program_refs": [
            {
                key: reference[key]
                for key in ("program_ref", "entrypoint", "target_agent_identity_requirement")
            }
            for reference in graph["imported_program_refs"]
        ],
    }


def _complete_frontend_shape(graph: dict) -> dict:
    """Project every typed semantic relation while ignoring ids and spans."""
    regions = {region["region_id"]: region for region in graph["regions"]}
    values = {value["value_id"]: value for value in graph["values"]}
    declarations = {
        declaration["decl_id"]: declaration for declaration in graph["declarations"]
    }
    slots_by_consumer: dict[str, list[str]] = {}
    for edge in graph["data_edges"]:
        slots_by_consumer.setdefault(edge["to_consumer"], []).append(
            edge["consumer_slot"]
        )

    def declaration_shape(declaration: dict) -> dict:
        return {
            key: declaration[key]
            for key in (
                "decl_kind",
                "input_type_ref",
                "output_type_ref",
                "target_ref",
                "context_default_present",
            )
            if key in declaration
        }

    def result_shape(node: dict) -> dict | None:
        result_id = node.get("result_value")
        if result_id is None:
            return None
        value = values[result_id]
        return {"type_ref": value["type_ref"], "origin": value["origin"]}

    calls = []
    for call in graph["call_intents"]:
        binding = declarations.get(call.get("binding_ref"))
        calls.append(
            {
                "intent_kind": call["intent_kind"],
                "binding": declaration_shape(binding) if binding else None,
                "receiver_kind": call.get("receiver_kind"),
                "parent_region_role": regions[call["parent_region_id"]]["region_role"],
                "operand_slots": sorted(slots_by_consumer.get(call["node_id"], [])),
                "result": result_shape(call),
            }
        )

    controls = []
    for control in graph["control_intents"]:
        controls.append(
            {
                "control_kind": control["control_kind"],
                "parent_region_role": regions[control["parent_region_id"]]["region_role"],
                "body_region_roles": [
                    regions[region_id]["region_role"]
                    for region_id in control.get("body_region_ids", [])
                ],
                "operand_slots": sorted(
                    slots_by_consumer.get(control["node_id"], [])
                ),
                "result": result_shape(control),
            }
        )

    return {
        "program": {
            key: graph["program_definitions"][0][key]
            for key in (
                "program_id",
                "input_type_ref",
                "output_type_ref",
                "context_type_ref",
                "has_default_context",
            )
        },
        "declarations": sorted(
            (declaration_shape(value) for value in declarations.values()),
            key=lambda value: json.dumps(value, sort_keys=True),
        ),
        "calls": calls,
        "controls": controls,
        "region_roles": sorted(region["region_role"] for region in regions.values()),
        "context_flow": sorted(
            edge["context_type_ref"] for edge in graph["context_flow"]
        ),
        "hooks": [
            {
                key: hook[key]
                for key in (
                    "scope",
                    "phase",
                    "input_type_ref",
                    "output_type_ref",
                    "return_mode",
                )
            }
            for hook in graph["hook_bindings"]
        ],
        "capability_requirements": graph["capability_requirements"],
        "model_requirements": graph["model_requirements"],
        "imported_program_refs": [
            {
                key: reference[key]
                for key in (
                    "program_ref",
                    "entrypoint",
                    "target_agent_identity_requirement",
                )
            }
            for reference in graph["imported_program_refs"]
        ],
    }


def test_conversational_examples_lower_equivalently() -> None:
    python = python_air()
    typescript = typescript_air()
    assert python["schema_version"] == "apxm.air.v1"
    assert typescript["schema_version"] == "apxm.air.v1"
    assert _canonical_shape(python) == _canonical_shape(typescript)


def test_conversational_examples_record_equivalent_frontend_graph_intent_and_diagnostics() -> None:
    assert _frontend_intent(python_graph()) == _frontend_intent(typescript_graph())
    assert python_diagnostics() is None
    assert typescript_diagnostics() is None


def test_conversational_example_uses_only_generic_operations() -> None:
    air = python_air()
    ops = {op["op"] for op in air["semantic_operations"]}
    assert ops == {"program.new", "program.invoke", "model.call"}
    structural = {node["kind"] for node in air["structural_ir"]}
    assert "ais.loop" in structural
    assert "yield" in structural


def test_paired_corpus_records_equivalent_complete_frontend_intent() -> None:
    python = _python_parity()
    typescript = _typescript_parity("frontendGraph")
    python_shape = _complete_frontend_shape(python)
    typescript_shape = _complete_frontend_shape(typescript)
    assert python_shape == typescript_shape, json.dumps(
        {"python": python_shape, "typescript": typescript_shape},
        indent=2,
        sort_keys=True,
    )
    assert _python_parity("--diagnostics") is None
    assert _typescript_parity("diagnostics") is None


def test_paired_corpus_covers_every_public_operation_and_structural_family() -> None:
    python = _python_parity("--air")
    typescript = _typescript_parity("canonicalAir")
    assert _canonical_shape(python) == _canonical_shape(typescript)
    assert {operation["op"] for operation in python["semantic_operations"]} == {
        "model.call",
        "capability.invoke",
        "program.new",
        "program.invoke",
        "await.event",
    }
    assert {
        "ais.loop",
        "branch",
        "parallel_join",
        "try",
        "yield",
    } <= {node["kind"] for node in python["structural_ir"]}
