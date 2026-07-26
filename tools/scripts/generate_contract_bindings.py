#!/usr/bin/env python3
"""Generate canonical agents bindings from the contracts repository."""

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
OUTPUT_PATHS = (
    Path("crates/tools/cli/generated/typescript/event-v1.ts"),
    Path("crates/tools/cli/generated/python/event_v1.py"),
    Path("crates/machine/contracts/src/events/generated_event_kind_registry.rs"),
    Path("crates/machine/contracts/src/types/generated_host_execution_manifest.rs"),
    Path("crates/machine/contracts/src/types/generated_context_contracts.rs"),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args()


def render(output_root: Path) -> int:
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
        return generated.returncode

    for relative_path in OUTPUT_PATHS:
        path = output_root / relative_path
        if not path.is_file():
            print(f"contract generator did not produce {relative_path}", file=sys.stderr)
            return 1
        if path.suffix == ".rs":
            formatted = subprocess.run(
                ["rustfmt", "--edition", "2024", str(path)],
                cwd=REPO_ROOT,
                check=False,
            )
            if formatted.returncode != 0:
                return formatted.returncode
    return 0


def check_outputs(output_root: Path) -> int:
    stale: list[Path] = []
    for relative_path in OUTPUT_PATHS:
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


def write_outputs(output_root: Path) -> None:
    for relative_path in OUTPUT_PATHS:
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
        result = render(output_root)
        if result != 0:
            return result
        if args.check:
            return check_outputs(output_root)
        write_outputs(output_root)
    return 0


if __name__ == "__main__":
    sys.exit(main())
