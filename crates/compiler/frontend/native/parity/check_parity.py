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
]


def python_air(builder_name: str) -> str:
    sys.path.insert(0, str(PYTHON_PKG))
    import apxm_program
    from apxm_program import example

    return apxm_program.canonical_air_json(getattr(example, builder_name)())


def typescript_air(argument: str) -> str:
    result = subprocess.run(
        ["node", "--experimental-strip-types", str(TS_JS / "emit_air.ts"), argument],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def main() -> int:
    for name, golden_path, builder_name, node_arg in EXAMPLES:
        golden = golden_path.read_text().strip()
        air_py = python_air(builder_name).strip()
        air_ts = typescript_air(node_arg).strip()
        if air_py != golden:
            print(f"FAIL: [{name}] Python AIR does not match the golden vector", file=sys.stderr)
            return 1
        if air_ts != golden:
            print(f"FAIL: [{name}] TypeScript AIR does not match the golden vector", file=sys.stderr)
            return 1
        if air_py != air_ts:
            print(f"FAIL: [{name}] Python and TypeScript AIR differ", file=sys.stderr)
            return 1
        print(f"OK: [{name}] Python and TypeScript lower to identical AIR ({len(golden)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
