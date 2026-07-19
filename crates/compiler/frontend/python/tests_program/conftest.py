"""Build the native authoring bridge once, in process, before the tests run.

Running ``pytest`` alone produces a clean, working install: the PyO3 extension is
compiled and placed inside the ``apxm_program`` package, so authoring lowers to
canonical AIR through the in-process bridge with no CLI or network step.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[5]
PACKAGE_DIR = REPO_ROOT / "crates" / "compiler" / "frontend" / "python"
NATIVE_DEST = PACKAGE_DIR / "apxm_program" / "_native.so"
CARGO = REPO_ROOT / "tools" / "scripts" / "cargo.py"


def _run(args: list[str]) -> str:
    env = dict(os.environ)
    env.setdefault("PYO3_PYTHON", sys.executable)
    result = subprocess.run(
        [sys.executable, str(CARGO), *args],
        cwd=str(REPO_ROOT),
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def _build_native() -> None:
    _run(["build", "-p", "apxm-frontend-python", "--release"])
    target_dir = Path(_run(["target-dir"]))
    built = target_dir / "release" / "lib_native.so"
    if not built.is_file():
        raise RuntimeError(f"native bridge not found at {built}")
    shutil.copy2(built, NATIVE_DEST)


def pytest_configure(config) -> None:  # noqa: ARG001
    if str(PACKAGE_DIR) not in sys.path:
        sys.path.insert(0, str(PACKAGE_DIR))
    # Default: build the bridge for a clean install. Setting
    # APXM_SKIP_NATIVE_BUILD=1 reuses an already-placed module (useful when the
    # shared cargo target directory is busy).
    if os.environ.get("APXM_SKIP_NATIVE_BUILD") == "1":
        if not NATIVE_DEST.is_file():
            raise RuntimeError(
                f"APXM_SKIP_NATIVE_BUILD=1 but {NATIVE_DEST} is missing; build it first"
            )
        return
    _build_native()
