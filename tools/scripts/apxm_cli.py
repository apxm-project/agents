#!/usr/bin/env python3
"""Run the APXM CLI through the Dekk-controlled Cargo environment."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]


def main(argv: list[str]) -> int:
    cargo_wrapper = REPOSITORY_ROOT / "tools" / "scripts" / "cargo.py"
    command = [
        sys.executable,
        str(cargo_wrapper),
        "run",
        "-p",
        "apxm-cli",
        "--bin",
        "apxm",
        "--features",
        "driver,metrics",
        "--release",
        "--",
        *argv,
    ]
    return subprocess.run(command, cwd=REPOSITORY_ROOT, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
