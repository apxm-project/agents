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
GOLDEN = PARITY_DIR / "air.expected.json"


def python_air() -> str:
    sys.path.insert(0, str(PYTHON_PKG))
    import apxm_program
    from apxm_program.example import specialist_graph

    return apxm_program.canonical_air_json(specialist_graph())


def typescript_air() -> str:
    result = subprocess.run(
        ["node", "--experimental-strip-types", str(TS_JS / "emit_air.ts")],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def main() -> int:
    golden = GOLDEN.read_text().strip()
    air_py = python_air().strip()
    air_ts = typescript_air().strip()

    if air_py != golden:
        print("FAIL: Python AIR does not match the golden vector", file=sys.stderr)
        return 1
    if air_ts != golden:
        print("FAIL: TypeScript AIR does not match the golden vector", file=sys.stderr)
        return 1
    if air_py != air_ts:
        print("FAIL: Python and TypeScript AIR differ", file=sys.stderr)
        return 1

    print(f"OK: Python and TypeScript lower the example to identical AIR ({len(golden)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
