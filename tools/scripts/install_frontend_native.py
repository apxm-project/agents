#!/usr/bin/env python3
"""Install built frontend native bridges into their package load paths.

Cargo emits platform-native shared-library suffixes (``.so``, ``.dylib``,
``.dll``). Package loaders expect stable names:

- Python: ``apxm_program/_native.so`` (PyO3)
- TypeScript: ``dist/_native.node`` (Node-API)

This helper copies the release artifact under the stable name so Dekk gates do
not hard-code one host's library suffix.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CARGO = REPO_ROOT / "tools" / "scripts" / "cargo.py"

PYTHON_CANDIDATES = ("lib_native.so", "lib_native.dylib", "lib_native.dll", "_native.so", "_native.dylib")
TYPESCRIPT_CANDIDATES = (
    "libapxm_frontend_typescript.so",
    "libapxm_frontend_typescript.dylib",
    "libapxm_frontend_typescript.dll",
    "apxm_frontend_typescript.so",
    "apxm_frontend_typescript.dylib",
)


def _target_release() -> Path:
    result = subprocess.run(
        [sys.executable, str(CARGO), "target-dir"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return Path(result.stdout.strip()) / "release"


def _first_existing(directory: Path, names: tuple[str, ...]) -> Path:
    for name in names:
        candidate = directory / name
        if candidate.is_file():
            return candidate
    listed = ", ".join(sorted(path.name for path in directory.glob("*native*")))
    raise SystemExit(
        f"error: none of {', '.join(names)} found under {directory}"
        + (f" (saw: {listed})" if listed else "")
    )


def install_python(release: Path) -> Path:
    source = _first_existing(release, PYTHON_CANDIDATES)
    destination = (
        REPO_ROOT / "crates" / "compiler" / "frontend" / "python" / "apxm_program" / "_native.so"
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    print(f"installed {source.name} -> {destination.relative_to(REPO_ROOT)}")
    return destination


def install_typescript(release: Path) -> Path:
    source = _first_existing(release, TYPESCRIPT_CANDIDATES)
    destination = (
        REPO_ROOT
        / "crates"
        / "compiler"
        / "frontend"
        / "typescript"
        / "dist"
        / "_native.node"
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    print(f"installed {source.name} -> {destination.relative_to(REPO_ROOT)}")
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "target",
        choices=("python", "typescript", "all"),
        help="Which frontend native bridge to install",
    )
    args = parser.parse_args()
    release = _target_release()
    if args.target in {"python", "all"}:
        install_python(release)
    if args.target in {"typescript", "all"}:
        install_typescript(release)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
