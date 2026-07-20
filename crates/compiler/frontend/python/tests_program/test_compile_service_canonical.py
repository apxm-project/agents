"""The canonical session-compile contract: a canonical-authored agent package's
session source runs through the ``apxm_program`` frontend and emits valid
canonical ``apxm.air.v1`` on stdout — exactly what ``apxm compile-service-canonical``
captures for the Server session family (this exercises the same package-entry
subprocess flow the Rust command wraps)."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

FIVE_OPS = {"model.call", "capability.invoke", "program.new", "program.invoke", "await.event"}
PACKAGE_DIR = Path(__file__).resolve().parents[1]
SESSION_ENTRY = Path(__file__).resolve().parent / "fixtures" / "canonical_session_agent" / "session.py"


def _compile_session_package_air() -> dict:
    env = dict(os.environ)
    env["PYTHONPATH"] = str(PACKAGE_DIR) + os.pathsep + env.get("PYTHONPATH", "")
    result = subprocess.run(
        [sys.executable, str(SESSION_ENTRY)],
        capture_output=True,
        text=True,
        env=env,
        check=True,
    )
    return json.loads(result.stdout)


def test_canonical_session_package_emits_valid_five_op_air_with_await_event():
    air = _compile_session_package_air()
    assert air["schema_version"] == "apxm.air.v1"
    ops = [op["op"] for op in air["semantic_operations"]]
    for op in ops:
        assert op in FIVE_OPS, op
    # The session parks on await.event each turn — the exact point the durable
    # ContinuationPort suspends on and a delivered turn resumes.
    assert "await.event" in ops, ops
    assert "model.call" in ops, ops


def test_canonical_session_package_air_is_deterministic():
    assert _compile_session_package_air() == _compile_session_package_air()
