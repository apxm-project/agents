#!/usr/bin/env python3
"""Cross-language authoring parity: the same example program lowers to identical
canonical AIR through the Python (PyO3) and TypeScript (Node-API) bridges, and
both equal the checked-in golden vector.

Both native modules must already be built and placed:
  - Python:     apxm_program/_native.so
  - TypeScript: typescript/js/_native.node
"""

from __future__ import annotations

import subprocess
import sys
import json
from pathlib import Path

PARITY_DIR = Path(__file__).resolve().parent
ROOT = PARITY_DIR.parents[4]
PYTHON_PKG = ROOT / "crates" / "compiler" / "frontend" / "python"
TS_JS = PARITY_DIR.parent / "typescript" / "js"

# Each example: (name, golden file, python builder, node emit_air.ts argument).
EXAMPLES = [
    ("specialist", PARITY_DIR / "air.expected.json", "specialist_graph", "specialist"),
    (
        "external-agent",
        PARITY_DIR / "air.external-agent.expected.json",
        "external_agent_graph",
        "external-agent",
    ),
    ("gao", PARITY_DIR / "air.gao.expected.json", "gao_conversational_graph", "gao"),
]


def python_air(builder_name: str) -> str:
    sys.path.insert(0, str(PYTHON_PKG))
    import apxm_program
    from apxm_program import example

    if builder_name == "gao_conversational_graph":
        return apxm_program.build_gao().canonical_air_json()
    return apxm_program.canonical_air_json(getattr(example, builder_name)())


def typescript_air(argument: str) -> str:
    result = subprocess.run(
        ["node", "--experimental-strip-types", str(TS_JS / "emit_air.ts"), argument],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def python_gao_artifact() -> str:
    sys.path.insert(0, str(PYTHON_PKG))
    import apxm_program

    artifact = apxm_program.compile_artifact(apxm_program.build_gao().build_graph())
    return json.dumps(artifact, separators=(",", ":"))


def normalized_air(value: str) -> dict:
    air = json.loads(value)
    air["source_map"]["source_language"] = "frontend"
    for span in air["source_map"]["node_spans"]:
        span["source_file"] = "frontend"
    return air


def main() -> int:
    for name, golden_path, builder_name, node_arg in EXAMPLES:
        golden = golden_path.read_text().strip()
        air_py = python_air(builder_name).strip()
        air_ts = typescript_air(node_arg).strip()
        if name == "gao":
            if normalized_air(air_py) != normalized_air(golden):
                print(f"FAIL: [{name}] Python AIR does not match the semantic golden", file=sys.stderr)
                return 1
            if normalized_air(air_ts) != normalized_air(golden):
                print(f"FAIL: [{name}] TypeScript AIR does not match the semantic golden", file=sys.stderr)
                return 1
        else:
            if air_py != golden:
                print(f"FAIL: [{name}] Python AIR does not match the golden vector", file=sys.stderr)
                return 1
            if air_ts != golden:
                print(f"FAIL: [{name}] TypeScript AIR does not match the golden vector", file=sys.stderr)
                return 1
            if air_py != air_ts:
                print(f"FAIL: [{name}] Python and TypeScript AIR differ", file=sys.stderr)
                return 1
        if name == "gao":
            artifact_py = python_gao_artifact()
            artifact_ts = typescript_air("gao-artifact")
            python_value = json.loads(artifact_py)
            typescript_value = json.loads(artifact_ts)
            if normalized_air(json.dumps(python_value["air"])) != normalized_air(
                json.dumps(typescript_value["air"])
            ):
                print("FAIL: [gao] public artifacts have different program semantics", file=sys.stderr)
                return 1
            for field in (
                "hook_bindings",
                "entrypoints",
                "artifact_semantic_requirements",
                "integrity_algorithm",
            ):
                if python_value.get(field) != typescript_value.get(field):
                    print(f"FAIL: [gao] public artifact field {field} differs", file=sys.stderr)
                    return 1
            if python_value.get("air") != json.loads(air_py):
                print("FAIL: [gao] executable artifact does not embed canonical AIR", file=sys.stderr)
                return 1
        print(f"OK: [{name}] Python and TypeScript lower to identical AIR ({len(golden)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
