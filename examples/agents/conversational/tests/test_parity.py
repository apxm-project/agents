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
