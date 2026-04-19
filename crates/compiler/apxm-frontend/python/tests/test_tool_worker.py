"""Tests for the NDJSON RPC tool worker subprocess.

Exercises:
  - 3+ concurrent calls
  - 1 cancel
  - 1 exception (tool raises)
  - 1 unknown handler_id
  - EOF graceful shutdown
  - timeout enforcement
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import tempfile
from typing import Any

import pytest

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_WORKER_MODULE = "apxm.tool_worker"
_PYTHON = sys.executable

# Path to the apxm package so subprocess can find it.
_APXM_PKG = os.path.normpath(
    os.path.join(os.path.dirname(__file__), os.pardir, os.pardir)
)


def _make_env() -> dict[str, str]:
    """Build an env dict that puts the apxm package on PYTHONPATH."""
    env = os.environ.copy()
    python_root = os.path.normpath(
        os.path.join(os.path.dirname(__file__), os.pardir)
    )
    existing = env.get("PYTHONPATH", "")
    env["PYTHONPATH"] = python_root + (os.pathsep + existing if existing else "")
    return env


async def _spawn_worker(
    manifest_path: str | None = None,
) -> asyncio.subprocess.Process:
    """Spawn the tool_worker as a subprocess."""
    cmd = [_PYTHON, "-m", _WORKER_MODULE]
    if manifest_path is not None:
        cmd.append(manifest_path)
    proc = await asyncio.create_subprocess_exec(
        *cmd,
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=_make_env(),
    )
    return proc


async def _send(proc: asyncio.subprocess.Process, msg: dict[str, Any]) -> None:
    """Send a single NDJSON line to the worker."""
    assert proc.stdin is not None
    line = json.dumps(msg, separators=(",", ":")) + "\n"
    proc.stdin.write(line.encode())
    await proc.stdin.drain()


async def _recv(proc: asyncio.subprocess.Process, timeout: float = 10.0) -> dict[str, Any]:
    """Read one NDJSON line from the worker's stdout."""
    assert proc.stdout is not None
    raw = await asyncio.wait_for(proc.stdout.readline(), timeout=timeout)
    assert raw, "unexpected EOF from worker"
    return json.loads(raw.decode())


async def _close(proc: asyncio.subprocess.Process) -> None:
    """Close stdin (EOF) and wait for the worker to exit."""
    assert proc.stdin is not None
    proc.stdin.close()
    try:
        await asyncio.wait_for(proc.wait(), timeout=5.0)
    except (asyncio.TimeoutError, TimeoutError):
        proc.kill()
        await proc.wait()


def _write_bootstrap_script(tmp: str) -> str:
    """Write a temporary Python script that sets up a stub registry, then
    imports and runs the worker.  This avoids needing tools.py to exist."""
    script = os.path.join(tmp, "_run_worker.py")
    with open(script, "w") as fh:
        fh.write(
            '''\
import sys, os, json, asyncio

# Ensure the apxm package is importable.
pkg_root = os.environ.get("PYTHONPATH", "")
if pkg_root:
    for p in pkg_root.split(os.pathsep):
        if p not in sys.path:
            sys.path.insert(0, p)

from apxm import tool_worker

# ---- Stub tools (stand-in for @tool decorated functions) ----

class _StubTool:
    """Minimal stand-in matching the FunctionTool interface expected by the worker."""
    def __init__(self, fn):
        self.fn = fn

    def validate_args(self, args):
        return args

def _add(a: int, b: int) -> int:
    return a + b

def _greet(name: str) -> str:
    return f"hello {name}"

def _concat(items: list) -> str:
    return ",".join(str(i) for i in items)

def _boom() -> None:
    raise ValueError("intentional kaboom")

async def _slow() -> str:
    await asyncio.sleep(300)
    return "done"

tool_worker._set_registry({
    "tool:add": _StubTool(_add),
    "tool:greet": _StubTool(_greet),
    "tool:concat": _StubTool(_concat),
    "tool:boom": _StubTool(_boom),
    "tool:slow": _StubTool(_slow),
})

# Run the main loop (reads manifest from argv[1] if present, but we
# already populated the registry so it's optional).
tool_worker.main()
'''
        )
    return script


async def _spawn_with_stubs(tmp: str) -> asyncio.subprocess.Process:
    """Spawn a worker subprocess that uses stub tools."""
    script = _write_bootstrap_script(tmp)
    proc = await asyncio.create_subprocess_exec(
        _PYTHON,
        script,
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env=_make_env(),
    )
    return proc


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


@pytest.mark.asyncio
async def test_concurrent_calls():
    """Three concurrent calls complete with correct results."""
    with tempfile.TemporaryDirectory() as tmp:
        proc = await _spawn_with_stubs(tmp)
        try:
            # Send three calls concurrently.
            await _send(proc, {
                "v": 1, "type": "call", "req_id": "c1",
                "tool_id": "tool:add", "args": {"a": 3, "b": 4},
            })
            await _send(proc, {
                "v": 1, "type": "call", "req_id": "c2",
                "tool_id": "tool:greet", "args": {"name": "world"},
            })
            await _send(proc, {
                "v": 1, "type": "call", "req_id": "c3",
                "tool_id": "tool:concat", "args": {"items": [1, 2, 3]},
            })

            results: dict[str, dict[str, Any]] = {}
            for _ in range(3):
                r = await _recv(proc)
                results[r["req_id"]] = r

            assert results["c1"]["ok"] is True
            assert results["c1"]["value"] == 7

            assert results["c2"]["ok"] is True
            assert results["c2"]["value"] == "hello world"

            assert results["c3"]["ok"] is True
            assert results["c3"]["value"] == "1,2,3"
        finally:
            await _close(proc)


@pytest.mark.asyncio
async def test_unknown_handler_id():
    """Calling a non-existent tool_id returns an error with kind=unknown_handler."""
    with tempfile.TemporaryDirectory() as tmp:
        proc = await _spawn_with_stubs(tmp)
        try:
            await _send(proc, {
                "v": 1, "type": "call", "req_id": "u1",
                "tool_id": "tool:nonexistent", "args": {},
            })
            r = await _recv(proc)
            assert r["req_id"] == "u1"
            assert r["ok"] is False
            assert r["error"]["kind"] == "unknown_handler"
        finally:
            await _close(proc)


@pytest.mark.asyncio
async def test_exception_in_tool():
    """Tool that raises an exception returns structured error."""
    with tempfile.TemporaryDirectory() as tmp:
        proc = await _spawn_with_stubs(tmp)
        try:
            await _send(proc, {
                "v": 1, "type": "call", "req_id": "e1",
                "tool_id": "tool:boom", "args": {},
            })
            r = await _recv(proc)
            assert r["req_id"] == "e1"
            assert r["ok"] is False
            assert r["error"]["kind"] == "ValueError"
            assert "kaboom" in r["error"]["message"]
            assert r["error"]["traceback"] != ""
        finally:
            await _close(proc)


@pytest.mark.asyncio
async def test_cancel():
    """Cancelling an in-flight call produces a cancelled error."""
    with tempfile.TemporaryDirectory() as tmp:
        proc = await _spawn_with_stubs(tmp)
        try:
            # Send a slow call, then cancel it.
            await _send(proc, {
                "v": 1, "type": "call", "req_id": "s1",
                "tool_id": "tool:slow", "args": {},
            })
            # Give the worker time to start the task.
            await asyncio.sleep(0.3)
            await _send(proc, {
                "v": 1, "type": "cancel", "req_id": "s1",
            })
            r = await _recv(proc, timeout=5.0)
            assert r["req_id"] == "s1"
            assert r["ok"] is False
            assert r["error"]["kind"] in ("cancelled", "CancelledError")
        finally:
            await _close(proc)


@pytest.mark.asyncio
async def test_eof_graceful_shutdown():
    """Closing stdin causes the worker to exit cleanly."""
    with tempfile.TemporaryDirectory() as tmp:
        proc = await _spawn_with_stubs(tmp)
        # Send one call, read the result, then close.
        await _send(proc, {
            "v": 1, "type": "call", "req_id": "g1",
            "tool_id": "tool:add", "args": {"a": 1, "b": 2},
        })
        r = await _recv(proc)
        assert r["ok"] is True
        assert r["value"] == 3

        await _close(proc)
        assert proc.returncode == 0


@pytest.mark.asyncio
async def test_timeout_enforcement():
    """A call with a very short deadline_ms times out."""
    with tempfile.TemporaryDirectory() as tmp:
        proc = await _spawn_with_stubs(tmp)
        try:
            await _send(proc, {
                "v": 1, "type": "call", "req_id": "t1",
                "tool_id": "tool:slow", "args": {},
                "deadline_ms": 100,
            })
            r = await _recv(proc)
            assert r["req_id"] == "t1"
            assert r["ok"] is False
            assert r["error"]["kind"] == "timeout"
        finally:
            await _close(proc)
