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

    _prepend_manifest_source_dirs(entries)

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


def _prepend_manifest_source_dirs(entries: list[dict[str, Any]]) -> None:
    """Make source-file-backed modules importable for artifact-only runs."""
    source_dirs: list[str] = []
    seen: set[str] = set()
    for entry in entries:
        source_file = entry.get(PYTHON_TOOL_MANIFEST_SOURCE_FILE)
        if not source_file:
            continue
        source_dir = str(Path(source_file).resolve().parent)
        if source_dir in seen or source_dir in sys.path:
            continue
        seen.add(source_dir)
        source_dirs.append(source_dir)

    for source_dir in reversed(source_dirs):
        sys.path.insert(0, source_dir)


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
    spec.loader.exec_module(module)
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


HOOK_PAYLOAD_KEY: Final[str] = "__apxm_hook__"


class _HookCall:
    """The guarded tool call passed to a pre/post_tool hook."""

    def __init__(self, name: str, args: dict[str, Any]) -> None:
        self.name = name
        self.args = args or {}


class _HookCtx:
    """Lifecycle-hook context. Control helpers return a decision dict the runtime
    applies; side-effect helpers are best-effort (the worker subprocess has no
    direct runtime-memory access — those degrade predictably)."""

    def __init__(self, payload: dict[str, Any]) -> None:
        self.remaining_budget = payload.get("remaining_budget")
        self._system = payload.get("system", "")
        # Recent conversation window the runtime pre-loads into the payload, so a
        # hook can recall context without a worker->runtime callback (the bridge
        # is unidirectional). The runtime applies any accumulated writes after the
        # hook returns (see `_writes`), so umem() works on the post_turn path.
        self._window = payload.get("window") or []
        self._writes: list[dict[str, Any]] = []

    def log(self, *args: Any) -> None:
        print("[hook]", *args, file=sys.stderr)
        return None

    def allow(self) -> dict[str, Any]:
        return {"decision": "allow"}

    def deny(self, reason: str = "") -> dict[str, Any]:
        return {"decision": "deny", "reason": reason}

    def edit_args(self, args: dict[str, Any]) -> dict[str, Any]:
        return {"decision": "edit_args", "args": args}

    def replace_result(self, result: Any) -> dict[str, Any]:
        return {"decision": "replace_result", "result": result}

    def prepend_system(self, text: str) -> dict[str, Any]:
        return {"decision": "prepend_system", "text": text}

    def set_system(self, text: str) -> dict[str, Any]:
        return {"decision": "set_system", "text": text}

    def read_agents_md(self) -> str:
        for candidate in ("AGENTS.md", "CLAUDE.md"):
            try:
                return Path(candidate).read_text()
            except OSError:
                continue
        return ""

    def recall_window(self, n: int = 4) -> str:
        # The runtime pre-loads the recent window into the payload (no callback
        # needed). Return the last-n turns joined; empty if none pre-loaded.
        if not self._window:
            return ""
        return "\n".join(str(t) for t in self._window[-n:])

    def umem(self, key: str, value: Any) -> None:
        # Accumulate a memory write; the runtime applies it after the hook
        # returns (carried back in the decision's `writes` list).
        self._writes.append({"key": key, "value": value})
        return None

    def summarize(self, text: Any) -> str:
        # Default heuristic summary (author may override with their own logic).
        return str(text)[:280]


def _invoke_hook(fn: Any, event: str, payload: dict[str, Any]) -> Any:
    ctx = _HookCtx(payload)
    if event in ("session_start", "pre_ask"):
        ret = fn(ctx)
    elif event == "pre_tool":
        call = payload.get("call", {})
        ret = fn(ctx, _HookCall(call.get("name", ""), call.get("args", {})))
    elif event == "post_tool":
        call = payload.get("call", {})
        ret = fn(ctx, _HookCall(call.get("name", ""), {}), payload.get("result"))
    elif event == "post_turn":
        ret = fn(ctx, payload.get("reply"))
    else:  # pre_turn and any future ctx-only event
        ret = fn(ctx)
    # Normalize to a decision dict and attach any accumulated memory writes so
    # side-effect helpers (ctx.umem) take effect even when the hook returns None.
    decision = ret if isinstance(ret, dict) else {}
    if ctx._writes and "writes" not in decision:
        decision = {**decision, "writes": ctx._writes}
    return decision


async def _handle_hook_call(req_id: str, tool: Any, payload: dict[str, Any]) -> None:
    """Invoke a lifecycle hook and emit its decision dict as the result value."""
    if tool is None:
        await _write_line_async(
            {
                WIRE_FIELD_VERSION: WIRE_VERSION,
                WIRE_FIELD_TYPE: WIRE_TYPE_RESULT,
                WIRE_FIELD_REQUEST_ID: req_id,
                WIRE_FIELD_OK: False,
                WIRE_FIELD_ERROR: {
                    WIRE_FIELD_ERROR_KIND: WIRE_ERROR_UNKNOWN_HANDLER,
                    WIRE_FIELD_MESSAGE: "no handler registered for hook",
                    WIRE_FIELD_ERROR_TRACEBACK: "",
                },
            }
        )
        return
    fn = tool.fn if hasattr(tool, "fn") else tool
    event = payload.get("event", "")
    try:
        loop = asyncio.get_running_loop()
        ret = await loop.run_in_executor(None, lambda: _invoke_hook(fn, event, payload))
        value = ret if isinstance(ret, dict) else {"decision": "allow"}
        await _write_line_async(
            {
                WIRE_FIELD_VERSION: WIRE_VERSION,
                WIRE_FIELD_TYPE: WIRE_TYPE_RESULT,
                WIRE_FIELD_REQUEST_ID: req_id,
                WIRE_FIELD_OK: True,
                WIRE_FIELD_VALUE: value,
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

    # Lifecycle-hook invocation (shares the tool path; constitution #4). The
    # payload carries the event + call/result; the hook returns a decision dict.
    if isinstance(args, dict) and HOOK_PAYLOAD_KEY in args:
        await _handle_hook_call(req_id, tool, args[HOOK_PAYLOAD_KEY])
        return

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
