"""Ensure retired Agent Program contract coordinates stay out of active surfaces."""

from __future__ import annotations

import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
RETIRED_COORDINATES = ("apxm.air.v1", "apxm.frontend-graph.v1")
ACTIVE_ROOTS = (
    ".dekk.toml",
    "contracts/schemas",
    "contracts/vectors",
    "crates/compiler",
    "crates/machine/program/src",
    "crates/runtime",
    "crates/tools",
    "examples",
    "tools/scripts",
)
EXEMPTIONS = {
    "contracts/vectors/apxm.air.v1-rejection.json",
    "contracts/vectors/apxm.frontend-graph.v1-rejection.json",
}


def test_retired_program_coordinates_are_absent_from_active_surfaces() -> None:
    tracked = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", *ACTIVE_ROOTS],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    findings: list[str] = []
    for relative in tracked:
        if relative in EXEMPTIONS:
            continue
        path = ROOT / relative
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for coordinate in RETIRED_COORDINATES:
            if coordinate in text:
                findings.append(f"{relative}: {coordinate}")
    assert not findings, "retired v1 coordinates remain active:\n" + "\n".join(findings)
