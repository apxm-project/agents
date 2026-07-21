#!/usr/bin/env python3
"""`dekk agents check-frontend-parity` — canonical frontend parity/drift gate.

Runs every check the canonical frontend parity invariant names so
FrontendGraph/AIR drift between Rust, Python, and TypeScript fails CI, not just
local runs:

1. Build and place the Python PyO3 canonical frontend bridge.
2. Build and place the TypeScript Node-API canonical frontend bridge.
3. Run the canonical Python and TypeScript authoring parity suites.
4. Run the cross-language parity harness against shared golden AIR.
5. Run the hand-authored-AIR regression guard.

Exit 0 = clean. Exit 1 = any step failed.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PYTHON_NATIVE_DEST = (
    REPO_ROOT / "crates" / "compiler" / "frontend" / "python" / "apxm_program" / "_native.so"
)
TYPESCRIPT_NATIVE_DEST = (
    REPO_ROOT / "crates" / "compiler" / "frontend" / "native" / "typescript" / "js" / "_native.node"
)
TYPESCRIPT_DIR = REPO_ROOT / "crates" / "compiler" / "frontend" / "typescript"
TYPESCRIPT_DEPENDENCIES_DIR = TYPESCRIPT_DIR / "node_modules"
TYPESCRIPT_PARITY_TEST = "test/canonical-air.test.ts"


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
        "Build Python canonical frontend bridge",
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-frontend-python", "--release"],
    )
    target_dir_result = subprocess.run(
        [sys.executable, "tools/scripts/cargo.py", "target-dir"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    target_dir = Path(target_dir_result.stdout.strip())
    python_native = target_dir / "release" / "lib_native.so"
    if not python_native.is_file():
        print(f"error: expected built Python native bridge at {python_native}", file=sys.stderr)
        return 1
    shutil.copy2(python_native, PYTHON_NATIVE_DEST)

    python_bin = shutil.which("pytest") and sys.executable or sys.executable
    ok &= _run(
        "Python canonical frontend AIR parity tests",
        [python_bin, "-m", "pytest", "crates/compiler/frontend/python/tests/test_air_parity.py", "-q"],
    )

    ok &= _run(
        "Build TypeScript canonical frontend bridge",
        [sys.executable, "tools/scripts/cargo.py", "build", "-p", "apxm-frontend-typescript", "--release"],
    )
    typescript_native = target_dir / "release" / "libapxm_frontend_typescript.so"
    if not typescript_native.is_file():
        print(f"error: expected built TypeScript native bridge at {typescript_native}", file=sys.stderr)
        return 1
    shutil.copy2(typescript_native, TYPESCRIPT_NATIVE_DEST)

    if TYPESCRIPT_DIR.is_dir() and TYPESCRIPT_DEPENDENCIES_DIR.is_dir():
        ok &= _run(
            "TypeScript canonical frontend AIR parity tests",
            ["npx", "vitest", "run", TYPESCRIPT_PARITY_TEST],
            cwd=TYPESCRIPT_DIR,
        )
    else:
        print(
            "\nerror: TypeScript frontend parity requires installed dependencies: "
            f"{TYPESCRIPT_DEPENDENCIES_DIR} (run `dekk agents install`).",
            file=sys.stderr,
        )
        ok = False

    ok &= _run(
        "Cross-language canonical AIR parity",
        [sys.executable, "crates/compiler/frontend/native/parity/check_parity.py"],
    )

    ok &= _run(
        "Hand-authored-AIR regression guard",
        [sys.executable, "tools/scripts/check_no_frontend_air_authoring.py"],
    )

    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
