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
import os
import shutil
import subprocess
import sys
import tempfile
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
MACHO_MAGICS = {
    b"\xfe\xed\xfa\xce",
    b"\xce\xfa\xed\xfe",
    b"\xfe\xed\xfa\xcf",
    b"\xcf\xfa\xed\xfe",
    b"\xca\xfe\xba\xbe",
    b"\xbe\xba\xfe\xca",
    b"\xca\xfe\xba\xbf",
    b"\xbf\xba\xfe\xca",
}


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


def _is_macho(path: Path) -> bool:
    """Return whether a native bridge has a Mach-O file header."""

    try:
        with path.open("rb") as stream:
            return stream.read(4) in MACHO_MAGICS
    except OSError:
        return False


def _resign_macos(destination: Path, identifier: str) -> None:
    """Re-sign a copied Mach-O bridge before it reaches its final path.

    ``codesign`` defaults the signing identifier to the basename of the file it
    signs. The bridge is signed while it still carries a randomized temporary
    name, so the default identifier — and therefore the signed bytes — differ on
    every install. The release manifest pins the bridge by digest, so that alone
    made a qualified cohort impossible: any gate that reinstalls the bridge
    invalidates the digest it was generated from. Sign under the stable final
    name instead, which makes identical input bytes produce identical output.
    """

    if sys.platform != "darwin" or not _is_macho(destination):
        return
    try:
        subprocess.run(
            [
                "codesign",
                "--force",
                "--sign",
                "-",
                "--identifier",
                identifier,
                str(destination),
            ],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        detail = ""
        if isinstance(error, subprocess.CalledProcessError):
            detail = (error.stderr or error.stdout or "").strip()
        if not detail:
            detail = str(error)
        raise SystemExit(
            f"error: unable to ad-hoc sign the native bridge {destination}: {detail}"
        ) from error


def _install_bridge(source: Path, destination: Path) -> Path:
    """Install a bridge without carrying source metadata into its load path."""

    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", suffix=".tmp", dir=destination.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output, source.open("rb") as input_stream:
            shutil.copyfileobj(input_stream, output)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, 0o755)
        _resign_macos(temporary, destination.name)
        os.replace(temporary, destination)
    finally:
        if temporary.exists() or temporary.is_symlink():
            temporary.unlink()
    return destination


def install_python(release: Path) -> Path:
    source = _first_existing(release, PYTHON_CANDIDATES)
    destination = (
        REPO_ROOT / "crates" / "compiler" / "frontend" / "python" / "apxm_program" / "_native.so"
    )
    _install_bridge(source, destination)
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
    _install_bridge(source, destination)
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
