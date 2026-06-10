#!/usr/bin/env python3
"""Prepare and publish APXM releases through Dekk."""

from __future__ import annotations

import sys

from apxm_release.cli import main


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
