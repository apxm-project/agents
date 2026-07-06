#!/usr/bin/env python3
"""Run the APXM CLI through the Dekk-controlled Cargo environment."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    project_root = Path(__file__).resolve().parents[2]
    cargo_wrapper = project_root / "tools" / "scripts" / "cargo.py"
    command = [
        sys.executable,
        str(cargo_wrapper),
        "run",
        "-p",
        "apxm-cli",
        "--features",
        "driver,metrics",
        "--release",
        "--",
        *argv,
    ]
    return subprocess.run(command, cwd=project_root, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
