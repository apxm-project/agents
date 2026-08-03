#!/usr/bin/env python3
"""`dekk agents check-frontend-parity` — canonical frontend parity/drift gate.

Runs every check the canonical frontend parity invariant names so
FrontendGraph/AIR drift between Rust, Python, and TypeScript fails CI, not just
local runs:

1. Test the packed Python frontend in a clean consumer.
2. Test the packed TypeScript frontend in a clean consumer.
3. Build, compile, and run the repository examples from installed packages.
   Cross-language comparison lives in
   ``examples/agents/conversational/tests/test_parity.py`` and requires
   closed-semantics AIR equality after removing language-local source maps
   plus projected FrontendGraph intent parity.
4. Run the hand-authored-AIR regression guard.

Exit 0 = clean. Exit 1 = any step failed.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]


def _run(description: str, command: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> bool:
    print(f"\n==> {description}\n    $ {' '.join(command)}", flush=True)
    result = subprocess.run(command, cwd=cwd or REPO_ROOT, env=env)
    ok = result.returncode == 0
    if not ok:
        print(f"FAILED: {description} (exit {result.returncode})", file=sys.stderr)
    return ok


def main() -> int:
    ok = True

    ok &= _run(
        "Packed Python generic frontend",
        ["dekk", "agents", "test-python-frontend"],
    )
    ok &= _run(
        "Packed TypeScript generic frontend",
        ["dekk", "agents", "test-typescript-frontend"],
    )
    ok &= _run(
        "Installed-package repository example parity",
        ["dekk", "agents", "test-frontend-examples"],
    )
    ok &= _run(
        "Hand-authored-AIR regression guard",
        [sys.executable, "tools/scripts/check_no_frontend_air_authoring.py"],
    )

    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
