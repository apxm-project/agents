#!/usr/bin/env python3
"""`dekk agents check-frontend-parity` — frontend-graph DTO parity/drift gate.

Runs every check `the frontend graph parity invariant` names so DTO/printer drift
between Rust, Python, and TypeScript fails CI, not just local runs:

1. Rust `frontend_graph`/`frontend_air` unit tests (DTO conversion + CLI
   surface, including the fixture-golden and wire-shape-parity tests).
2. Builds the `apxm` CLI (debug profile, fast) and points `APXM_BIN` at it
   so the Python/TypeScript parity tests actually shell out and compare
   real output instead of skipping.
3. `test_air_parity.py` (pytest) and `emit-air.test.ts` (vitest) golden
   comparisons, now un-skipped.
4. The hand-authored-AIR regression guard
   (`check_no_frontend_air_authoring.py`).

Exit 0 = clean. Exit 1 = any step failed.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
TYPESCRIPT_DIR = REPO_ROOT / "crates" / "compiler" / "frontend" / "typescript"


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
        "Rust frontend-graph DTO tests (apxm-compiler)",
        [sys.executable, "tools/scripts/cargo.py", "test", "-p", "apxm-compiler", "frontend_graph::"],
    )
    ok &= _run(
        "Rust frontend-air CLI + parity tests (apxm-cli)",
        [sys.executable, "tools/scripts/cargo.py", "test", "-p", "apxm-cli", "--bin", "apxm", "frontend_air::"],
    )

    ok &= _run(
        "Build apxm CLI (debug) for live parity tests",
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-cli", "--bin", "apxm"],
    )
    target_dir_result = subprocess.run(
        [sys.executable, "tools/scripts/cargo.py", "target-dir"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    target_dir = Path(target_dir_result.stdout.strip())
    apxm_bin = target_dir / "debug" / "apxm"
    if not apxm_bin.is_file():
        print(f"error: expected built apxm binary at {apxm_bin}", file=sys.stderr)
        return 1

    live_env = dict(os.environ)
    live_env["APXM_BIN"] = str(apxm_bin)

    python_bin = shutil.which("pytest") and sys.executable or sys.executable
    ok &= _run(
        "Python frontend AIR parity tests (pytest, APXM_BIN set)",
        [python_bin, "-m", "pytest", "crates/compiler/frontend/python/tests/test_air_parity.py", "-q"],
        env=live_env,
    )

    if TYPESCRIPT_DIR.exists() and (TYPESCRIPT_DIR / "node_modules").exists():
        ok &= _run(
            "TypeScript frontend AIR parity tests (vitest, APXM_BIN set)",
            ["npx", "vitest", "run", "test/emit-air.test.ts"],
            cwd=TYPESCRIPT_DIR,
            env=live_env,
        )
    else:
        print(
            "\n==> Skipping TypeScript parity tests: "
            f"{TYPESCRIPT_DIR / 'node_modules'} not installed (run `npm install` in "
            "crates/compiler/frontend/typescript first)."
        )

    ok &= _run(
        "Hand-authored-AIR regression guard",
        [sys.executable, "tools/scripts/check_no_frontend_air_authoring.py"],
    )

    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
