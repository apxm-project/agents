"""Ensure retired Agent Program contract coordinates stay out of active surfaces."""

from __future__ import annotations

import os
import subprocess
import sys
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
# These two vector files carry inline rejection cases (negative fixtures) that
# must name the retired coordinate as the thing under test. The former
# standalone `*-v1-rejection.json` files were retired in the de-versioning
# sweep; their cases now live inline here instead.
EXEMPTIONS = {
    "contracts/vectors/apxm.air.json",
    "contracts/vectors/apxm.frontend-graph.json",
}


def test_retired_program_coordinates_are_absent_from_active_surfaces() -> None:
    # `cwd=` rather than `git -C`: the latter needs git >= 1.8.5, and this repo
    # is developed on hosts carrying git 1.8.3.1, where `-C` is a usage error.
    git_env = os.environ.copy()
    if sys.platform == "darwin":
        # Dekk's conda libiconv must not interpose on the host Git ABI.
        git_env.pop("DYLD_LIBRARY_PATH", None)
        git_env.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
    tracked = subprocess.run(
        ["git", "ls-files", *ACTIVE_ROOTS],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        env=git_env,
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
        except FileNotFoundError:
            # Tracked but deleted in the working tree: a removed file carries no
            # active surface, so it cannot hold a retired coordinate.
            continue
        for coordinate in RETIRED_COORDINATES:
            if coordinate in text:
                findings.append(f"{relative}: {coordinate}")
    assert not findings, "retired v1 coordinates remain active:\n" + "\n".join(findings)
