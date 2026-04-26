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
import importlib.util
import json
import sys
import traceback
from pathlib import Path
from typing import Any, Final

from apxm.constants import (
    PYTHON_TOOL_MANIFEST_HANDLER_ID,
    PYTHON_TOOL_MANIFEST_MODULE,
    PYTHON_TOOL_MANIFEST_QUALNAME,
    PYTHON_TOOL_MANIFEST_SOURCE_FILE,
)

# Protocol version understood by this worker.
WIRE_VERSION: Final[int] = 1
WIRE_FIELD_VERSION: Final[str] = "v"
WIRE_FIELD_TYPE: Final[str] = "type"
WIRE_FIELD_LEVEL: Final[str] = "level"
WIRE_FIELD_MESSAGE: Final[str] = "message"
WIRE_FIELD_REQUEST_ID: Final[str] = "req_id"
WIRE_FIELD_TOOL_ID: Final[str] = "tool_id"
WIRE_FIELD_ARGS: Final[str] = "args"
WIRE_FIELD_DEADLINE_MS: Final[str] = "deadline_ms"
WIRE_FIELD_OK: Final[str] = "ok"
WIRE_FIELD_VALUE: Final[str] = "value"
WIRE_FIELD_ERROR: Final[str] = "error"
WIRE_FIELD_ERROR_KIND: Final[str] = "kind"
WIRE_FIELD_ERROR_TRACEBACK: Final[str] = "traceback"

WIRE_TYPE_CALL: Final[str] = "call"
WIRE_TYPE_CANCEL: Final[str] = "cancel"
WIRE_TYPE_LOG: Final[str] = "log"
WIRE_TYPE_RESULT: Final[str] = "result"

WIRE_LEVEL_ERROR: Final[str] = "error"

WIRE_ERROR_UNKNOWN_HANDLER: Final[str] = "unknown_handler"
WIRE_ERROR_TIMEOUT: Final[str] = "timeout"
WIRE_ERROR_CANCELLED: Final[str] = "cancelled"

# ---------------------------------------------------------------------------
# Registry interface
# ---------------------------------------------------------------------------

# Lazy import: tools.py may not exist yet during development.  The worker
# only needs ``_TOOL_REGISTRY`` (dict[str, FunctionTool]) where each value
# has ``.fn`` (callable) and ``.validate_args(args)`` (optional).
_registry: dict[str, Any] | None = None
MODULE_MAIN = "__main__"
SCRIPT_MODULE_PREFIX = "_apxm_tool_script_"


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
        module_name = entry[PYTHON_TOOL_MANIFEST_MODULE]
        try:
            module = _import_tool_module(entry)
            _ensure_manifest_handler(module, entry)
        except Exception as exc:
            _write_line(
                {
                    WIRE_FIELD_VERSION: WIRE_VERSION,
                    WIRE_FIELD_TYPE: WIRE_TYPE_LOG,
                    WIRE_FIELD_LEVEL: WIRE_LEVEL_ERROR,
                    WIRE_FIELD_MESSAGE: f"failed to import {module_name}: {exc}",
                }
            )


def _import_tool_module(entry: dict[str, Any]) -> Any:
    module_name = entry[PYTHON_TOOL_MANIFEST_MODULE]
    source_file = entry.get(PYTHON_TOOL_MANIFEST_SOURCE_FILE)
    if module_name != MODULE_MAIN or not source_file:
        return importlib.import_module(module_name)

    source_path = Path(source_file).resolve()
    synthetic_name = f"{SCRIPT_MODULE_PREFIX}{source_path.stem}"
    loaded = sys.modules.get(synthetic_name)
    if loaded is not None:
        return loaded

    spec = importlib.util.spec_from_file_location(synthetic_name, source_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load tool source file {source_path}")

    module = importlib.util.module_from_spec(spec)
    sys.modules[synthetic_name] = module
    source_dir = str(source_path.parent)
    inserted_source_dir = False
    if source_dir not in sys.path:
        sys.path.insert(0, source_dir)
        inserted_source_dir = True
    try:
        spec.loader.exec_module(module)
    finally:
        if inserted_source_dir:
            try:
                sys.path.remove(source_dir)
            except ValueError:
                pass
    return module


def _resolve_qualname(module: Any, qualname: str) -> Any:
    target = module
    for part in qualname.split("."):
        if part == "<locals>":
            raise AttributeError(f"cannot resolve local tool qualname {qualname!r}")
        target = getattr(target, part)
    return target


def _ensure_manifest_handler(module: Any, entry: dict[str, Any]) -> None:
    registry = _get_registry()
    handler_id = entry[PYTHON_TOOL_MANIFEST_HANDLER_ID]
    if handler_id in registry:
        return
    registry[handler_id] = _resolve_qualname(module, entry[PYTHON_TOOL_MANIFEST_QUALNAME])


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
    req_id: str = msg[WIRE_FIELD_REQUEST_ID]
    tool_id: str = msg[WIRE_FIELD_TOOL_ID]
    args: dict[str, Any] = msg.get(WIRE_FIELD_ARGS, {})

    registry = _get_registry()
    tool = registry.get(tool_id)

    if tool is None:
        await _write_line_async(
            {
                WIRE_FIELD_VERSION: WIRE_VERSION,
                WIRE_FIELD_TYPE: WIRE_TYPE_RESULT,
                WIRE_FIELD_REQUEST_ID: req_id,
                WIRE_FIELD_OK: False,
                WIRE_FIELD_ERROR: {
                    WIRE_FIELD_ERROR_KIND: WIRE_ERROR_UNKNOWN_HANDLER,
                    WIRE_FIELD_MESSAGE: f"no handler registered for tool_id={tool_id!r}",
                    WIRE_FIELD_ERROR_TRACEBACK: "",
                },
            }
        )
        return

    deadline_ms = msg.get(WIRE_FIELD_DEADLINE_MS)
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
                WIRE_FIELD_VERSION: WIRE_VERSION,
                WIRE_FIELD_TYPE: WIRE_TYPE_RESULT,
                WIRE_FIELD_REQUEST_ID: req_id,
                WIRE_FIELD_OK: True,
                WIRE_FIELD_VALUE: value,
            }
        )
    except asyncio.TimeoutError:
        await _write_line_async(
            {
                WIRE_FIELD_VERSION: WIRE_VERSION,
                WIRE_FIELD_TYPE: WIRE_TYPE_RESULT,
                WIRE_FIELD_REQUEST_ID: req_id,
                WIRE_FIELD_OK: False,
                WIRE_FIELD_ERROR: {
                    WIRE_FIELD_ERROR_KIND: WIRE_ERROR_TIMEOUT,
                    WIRE_FIELD_MESSAGE: f"tool call exceeded deadline ({deadline_ms}ms)",
                    WIRE_FIELD_ERROR_TRACEBACK: "",
                },
            }
        )
    except asyncio.CancelledError:
        # Emit a structured cancelled result instead of letting the error
        # propagate and drop the response.
        # Use sync write since the event loop may be tearing down.
        _write_line(
            {
                WIRE_FIELD_VERSION: WIRE_VERSION,
                WIRE_FIELD_TYPE: WIRE_TYPE_RESULT,
                WIRE_FIELD_REQUEST_ID: req_id,
                WIRE_FIELD_OK: False,
                WIRE_FIELD_ERROR: {
                    WIRE_FIELD_ERROR_KIND: WIRE_ERROR_CANCELLED,
                    WIRE_FIELD_MESSAGE: "call was cancelled",
                    WIRE_FIELD_ERROR_TRACEBACK: "",
                },
            }
        )
    except Exception as exc:
        await _write_line_async(
            {
                WIRE_FIELD_VERSION: WIRE_VERSION,
                WIRE_FIELD_TYPE: WIRE_TYPE_RESULT,
                WIRE_FIELD_REQUEST_ID: req_id,
                WIRE_FIELD_OK: False,
                WIRE_FIELD_ERROR: {
                    WIRE_FIELD_ERROR_KIND: type(exc).__name__,
                    WIRE_FIELD_MESSAGE: str(exc),
                    WIRE_FIELD_ERROR_TRACEBACK: traceback.format_exc(),
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

        msg_type = msg.get(WIRE_FIELD_TYPE)
        req_id = msg.get(WIRE_FIELD_REQUEST_ID)

        if msg_type == WIRE_TYPE_CALL and req_id is not None:
            task = asyncio.create_task(_handle_call(msg))
            inflight[req_id] = task
            task.add_done_callback(lambda t, rid=req_id: inflight.pop(rid, None))

        elif msg_type == WIRE_TYPE_CANCEL and req_id is not None:
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
