"""NDJSON RPC subprocess worker for Python tool dispatch.

Invoked as::

    python -m apxm.tool_worker <manifest.json>

The manifest is a JSON array of ``{"handler_id", "module", "qualname"}``
entries.  On startup each module is imported so that ``@tool``-decorated
functions register themselves in ``_TOOL_REGISTRY``.

Wire protocol (NDJSON over stdin/stdout, multiplexed via ``req_id``):

    Request::

        {"v":1,"type":"call","req_id":"u-1",
         "tool_id":"sha256:...","args":{...},"deadline_ms":9500}

    Result OK::

        {"v":1,"type":"result","req_id":"u-1","ok":true,"value":...}

    Result err::

        {"v":1,"type":"result","req_id":"u-1","ok":false,
         "error":{"kind":"...","message":"...","traceback":"..."}}

    Cancel::

        {"v":1,"type":"cancel","req_id":"u-1"}
"""

from __future__ import annotations

import asyncio
import importlib
import json
import sys
import traceback
from typing import Any

# Protocol version understood by this worker.
_WIRE_VERSION = 1

# ---------------------------------------------------------------------------
# Registry interface
# ---------------------------------------------------------------------------

# Lazy import: tools.py may not exist yet during development.  The worker
# only needs ``_TOOL_REGISTRY`` (dict[str, FunctionTool]) where each value
# has ``.fn`` (callable) and ``.validate_args(args)`` (optional).
_registry: dict[str, Any] | None = None


def _get_registry() -> dict[str, Any]:
    """Return the global tool registry, importing tools.py lazily."""
    global _registry
    if _registry is None:
        try:
            from apxm.tools import _TOOL_REGISTRY

            _registry = _TOOL_REGISTRY
        except ImportError:
            _registry = {}
    return _registry


def _set_registry(reg: dict[str, Any]) -> None:
    """Override the registry (used by tests)."""
    global _registry
    _registry = reg


# ---------------------------------------------------------------------------
# Manifest loading
# ---------------------------------------------------------------------------


def _load_manifest(path: str) -> None:
    """Import every module listed in *path* so decorators populate the registry."""
    with open(path) as fh:
        entries = json.load(fh)

    for entry in entries:
        module_name = entry["module"]
        try:
            importlib.import_module(module_name)
        except Exception as exc:
            _write_line(
                {
                    "v": _WIRE_VERSION,
                    "type": "log",
                    "level": "error",
                    "message": f"failed to import {module_name}: {exc}",
                }
            )


# ---------------------------------------------------------------------------
# I/O helpers
# ---------------------------------------------------------------------------

_stdout_lock = asyncio.Lock()


async def _write_line_async(obj: dict[str, Any]) -> None:
    """Serialize *obj* as a single JSON line to stdout (async-safe)."""
    line = json.dumps(obj, separators=(",", ":"), default=str) + "\n"
    async with _stdout_lock:
        sys.stdout.write(line)
        sys.stdout.flush()


def _write_line(obj: dict[str, Any]) -> None:
    """Synchronous variant used during startup (before the event loop)."""
    line = json.dumps(obj, separators=(",", ":"), default=str) + "\n"
    sys.stdout.write(line)
    sys.stdout.flush()


# ---------------------------------------------------------------------------
# Request dispatch
# ---------------------------------------------------------------------------


async def _handle_call(msg: dict[str, Any]) -> None:
    """Execute a tool call and emit the result.

    CancelledError is caught internally so that cancellation always emits a
    structured result rather than silently dropping the request.
    """
    req_id: str = msg["req_id"]
    tool_id: str = msg["tool_id"]
    args: dict[str, Any] = msg.get("args", {})

    registry = _get_registry()
    tool = registry.get(tool_id)

    if tool is None:
        await _write_line_async(
            {
                "v": _WIRE_VERSION,
                "type": "result",
                "req_id": req_id,
                "ok": False,
                "error": {
                    "kind": "unknown_handler",
                    "message": f"no handler registered for tool_id={tool_id!r}",
                    "traceback": "",
                },
            }
        )
        return

    deadline_ms = msg.get("deadline_ms")
    try:
        # Re-validate args if the tool exposes a validator (defense in depth).
        fn = tool.fn if hasattr(tool, "fn") else tool
        if hasattr(tool, "validate_args"):
            args = tool.validate_args(args)

        timeout = deadline_ms / 1000.0 if deadline_ms else None

        # Execute the tool function.  Sync functions are run in the default
        # executor so they don't block the event loop.
        if asyncio.iscoroutinefunction(fn):
            coro = fn(**args)
        else:
            loop = asyncio.get_running_loop()
            coro = loop.run_in_executor(None, lambda: fn(**args))

        if timeout is not None:
            value = await asyncio.wait_for(asyncio.shield(coro), timeout=timeout)
        else:
            value = await coro

        await _write_line_async(
            {
                "v": _WIRE_VERSION,
                "type": "result",
                "req_id": req_id,
                "ok": True,
                "value": value,
            }
        )
    except asyncio.TimeoutError:
        await _write_line_async(
            {
                "v": _WIRE_VERSION,
                "type": "result",
                "req_id": req_id,
                "ok": False,
                "error": {
                    "kind": "timeout",
                    "message": f"tool call exceeded deadline ({deadline_ms}ms)",
                    "traceback": "",
                },
            }
        )
    except asyncio.CancelledError:
        # Emit a structured cancelled result instead of letting the error
        # propagate and drop the response.
        # Use sync write since the event loop may be tearing down.
        _write_line(
            {
                "v": _WIRE_VERSION,
                "type": "result",
                "req_id": req_id,
                "ok": False,
                "error": {
                    "kind": "cancelled",
                    "message": "call was cancelled",
                    "traceback": "",
                },
            }
        )
    except Exception as exc:
        await _write_line_async(
            {
                "v": _WIRE_VERSION,
                "type": "result",
                "req_id": req_id,
                "ok": False,
                "error": {
                    "kind": type(exc).__name__,
                    "message": str(exc),
                    "traceback": traceback.format_exc(),
                },
            }
        )


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------


async def _run(manifest_path: str | None = None) -> None:
    """Read NDJSON from stdin, dispatch calls concurrently, handle cancels."""
    if manifest_path is not None:
        _load_manifest(manifest_path)

    # In-flight tasks keyed by req_id.
    inflight: dict[str, asyncio.Task[None]] = {}

    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader()
    protocol = asyncio.StreamReaderProtocol(reader)
    await loop.connect_read_pipe(lambda: protocol, sys.stdin)

    while True:
        raw = await reader.readline()
        if not raw:
            # EOF on stdin -> graceful shutdown.
            break

        line = raw.decode("utf-8", errors="replace").strip()
        if not line:
            continue

        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue

        msg_type = msg.get("type")
        req_id = msg.get("req_id")

        if msg_type == "call" and req_id is not None:
            task = asyncio.create_task(_handle_call(msg))
            inflight[req_id] = task
            task.add_done_callback(lambda t, rid=req_id: inflight.pop(rid, None))

        elif msg_type == "cancel" and req_id is not None:
            task = inflight.get(req_id)
            if task is not None and not task.done():
                task.cancel()

    # Cancel any remaining in-flight tasks on shutdown.
    for task in inflight.values():
        if not task.done():
            task.cancel()

    if inflight:
        await asyncio.gather(*inflight.values(), return_exceptions=True)


def main() -> None:
    """Entry point for ``python -m apxm.tool_worker``."""
    manifest_path = sys.argv[1] if len(sys.argv) > 1 else None
    asyncio.run(_run(manifest_path))


if __name__ == "__main__":
    main()
