#!/usr/bin/env python3
"""Generate canonical agents bindings from the contracts repository.

The owning generator (../contracts/tools/codegen.py) is not itself a fixpoint
for two of its Rust emitters: `render_agents_manifest` reorders the `use`
block relative to rustfmt's import-grouping order, and
`render_context_rust_types` emits an over-width `pub const ...
SCHEMA_VERSION` line that rustfmt line-wraps. This wrapper runs rustfmt over
the Rust outputs after invoking the owning generator so `dekk agents gen` is
a fixpoint here. It is a consumer-side workaround, not a second owner of
formatting: once Contracts makes those two emitters produce rustfmt-stable
output directly, this script's rustfmt pass becomes redundant and must be
deleted along with the `--check` byte-diff step below, leaving a thin
delegation to the owning generator.
"""

from __future__ import annotations

import argparse
import difflib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
CONTRACT_GENERATOR = REPO_ROOT.parent / "contracts/tools/codegen.py"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args()


def discover_output_paths(output_root: Path) -> tuple[Path, ...]:
    """Enumerate what the owning generator actually wrote, rather than a frozen copy.

    The owning generator (contracts/tools/codegen.py) has no `--list-outputs`
    reporting mode, so the only way to learn its exact output set without
    duplicating owner knowledge is to run it and see what appeared. A hardcoded
    tuple drifts silently the moment the owner adds or renames a file; walking
    the rendered tree cannot.
    """
    return tuple(
        sorted(
            path.relative_to(output_root)
            for path in output_root.rglob("*")
            if path.is_file()
        )
    )


def render(output_root: Path) -> tuple[int, tuple[Path, ...]]:
    generated = subprocess.run(
        [
            sys.executable,
            str(CONTRACT_GENERATOR),
            "--target",
            "agents",
            "--target-root",
            str(output_root),
        ],
        cwd=REPO_ROOT,
        check=False,
    )
    if generated.returncode != 0:
        return generated.returncode, ()

    output_paths = discover_output_paths(output_root)
    if not output_paths:
        print("contract generator produced no agents output", file=sys.stderr)
        return 1, ()

    for relative_path in output_paths:
        path = output_root / relative_path
        if path.suffix == ".rs":
            formatted = subprocess.run(
                ["rustfmt", "--edition", "2024", str(path)],
                cwd=REPO_ROOT,
                check=False,
            )
            if formatted.returncode != 0:
                return formatted.returncode, ()
    return 0, output_paths


def check_outputs(output_root: Path, output_paths: tuple[Path, ...]) -> int:
    stale: list[Path] = []
    for relative_path in output_paths:
        expected_path = output_root / relative_path
        current_path = REPO_ROOT / relative_path
        if not current_path.is_file():
            print(f"missing generated output: {relative_path}", file=sys.stderr)
            stale.append(relative_path)
            continue

        expected = expected_path.read_text(encoding="utf-8")
        current = current_path.read_text(encoding="utf-8")
        if current == expected:
            continue
        stale.append(relative_path)
        sys.stderr.writelines(
            difflib.unified_diff(
                current.splitlines(keepends=True),
                expected.splitlines(keepends=True),
                fromfile=str(relative_path),
                tofile=f"{relative_path} (generated)",
            )
        )

    if stale:
        paths = ", ".join(str(path) for path in stale)
        print(
            f"generated agents contract output is stale: run `dekk agents gen` ({paths})",
            file=sys.stderr,
        )
        return 1
    print("checked canonical agents contract bindings")
    return 0


def write_outputs(output_root: Path, output_paths: tuple[Path, ...]) -> None:
    for relative_path in output_paths:
        destination = REPO_ROOT / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(output_root / relative_path, destination)
    print("generated canonical agents contract bindings")


def main() -> int:
    args = parse_args()
    if not CONTRACT_GENERATOR.is_file():
        print(f"contracts generator not found: {CONTRACT_GENERATOR}", file=sys.stderr)
        return 1

    artifact_root = REPO_ROOT / ".apxm"
    artifact_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="contract-codegen-", dir=artifact_root) as temporary:
        output_root = Path(temporary)
        returncode, output_paths = render(output_root)
        if returncode != 0:
            return returncode
        if args.check:
            return check_outputs(output_root, output_paths)
        write_outputs(output_root, output_paths)
    return 0


if __name__ == "__main__":
    sys.exit(main())
