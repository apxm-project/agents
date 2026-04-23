#!/usr/bin/env python3
"""Compatibility shim that forwards to `dekk apxm ...`."""

from __future__ import annotations

import os
import sys

from bootstrap_dekk import ensure_dekk_bootstrap


def main() -> None:
    ensure_dekk_bootstrap()
    os.execvp(
        sys.executable,
        [sys.executable, "-m", "dekk.cli.main", "apxm", *sys.argv[1:]],
    )


if __name__ == "__main__":
    main()
