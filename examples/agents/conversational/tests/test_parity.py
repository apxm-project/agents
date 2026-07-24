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


def test_conversational_examples_lower_equivalently() -> None:
    python = python_air()
    typescript = typescript_air()
    assert python["schema_version"] == "apxm.air.v1"
    assert typescript["schema_version"] == "apxm.air.v1"
    assert _canonical_shape(python) == _canonical_shape(typescript)


def test_conversational_example_uses_only_generic_operations() -> None:
    air = python_air()
    ops = {op["op"] for op in air["semantic_operations"]}
    assert ops == {"program.new", "program.invoke", "model.call"}
    structural = {node["kind"] for node in air["structural_ir"]}
    assert "ais.loop" in structural
    assert "yield" in structural
