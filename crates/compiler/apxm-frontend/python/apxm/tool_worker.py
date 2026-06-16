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
import itertools
import json
import queue
import sys
import threading
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
# Worker-initiated calls back into the runtime (e.g. a hook's ctx.summarize ->
# llm.ask). The worker emits a host_call and blocks until the runtime replies
# with a host_result correlated by the callback req_id.
WIRE_TYPE_HOST_CALL: Final[str] = "host_call"
WIRE_TYPE_HOST_RESULT: Final[str] = "host_result"
WIRE_FIELD_PARENT_REQUEST_ID: Final[str] = "parent_req_id"
WIRE_FIELD_METHOD: Final[str] = "method"
WIRE_FIELD_PARAMS: Final[str] = "params"
HOST_METHOD_LLM_ASK: Final[str] = "llm.ask"
HOST_METHOD_TOOL_CALL: Final[str] = "tool.call"
HOST_METHOD_MEM_READ: Final[str] = "mem.read"
HOST_METHOD_MEM_RECENT: Final[str] = "mem.recent"

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

# A single threading lock guards ALL stdout writes. Hooks run in worker threads
# (run_in_executor) and may emit host_calls concurrently with the event loop's
# result frames, so the lock must be thread-safe (not an asyncio.Lock).
_stdout_thread_lock = threading.Lock()


def _emit_line(obj: dict[str, Any]) -> None:
    """Write *obj* as one NDJSON line to stdout, thread-safe across loop+threads."""
    line = json.dumps(obj, separators=(",", ":"), default=str) + "\n"
    with _stdout_thread_lock:
        sys.stdout.write(line)
        sys.stdout.flush()


async def _write_line_async(obj: dict[str, Any]) -> None:
    """Serialize *obj* as a single JSON line to stdout (async-safe)."""
    _emit_line(obj)


def _write_line(obj: dict[str, Any]) -> None:
    """Synchronous variant used during startup (before the event loop)."""
    _emit_line(obj)


# Pending host calls a hook raised, keyed by callback req_id. The hook thread
# blocks on the queue; the async _run loop delivers the host_result.
_host_pending: dict[str, "queue.Queue[tuple[bool, Any, str | None]]"] = {}
_host_pending_lock = threading.Lock()
_host_call_counter = itertools.count(1)
# Safety cap so a hook thread never blocks forever if the runtime never replies
# (the runtime also bounds the whole hook via HOOK_DEADLINE).
_HOST_CALL_TIMEOUT_S: Final[float] = 120.0


def _deliver_host_result(msg: dict[str, Any]) -> None:
    """Route a host_result frame to the waiting hook thread."""
    req_id = msg.get(WIRE_FIELD_REQUEST_ID)
    if req_id is None:
        return
    with _host_pending_lock:
        q = _host_pending.get(req_id)
    if q is None:
        return
    ok = bool(msg.get(WIRE_FIELD_OK, False))
    value = msg.get(WIRE_FIELD_VALUE)
    error = (msg.get(WIRE_FIELD_ERROR) or {}).get(WIRE_FIELD_MESSAGE)
    q.put((ok, value, error))


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

    def __init__(self, payload: dict[str, Any], req_id: str = "") -> None:
        self.remaining_budget = payload.get("remaining_budget")
        # The parent call's req_id, stamped on host_calls this hook raises.
        self._req_id = req_id
        self._system = payload.get("system", "")
        # Accumulated memory writes; the runtime applies them after the hook
        # returns (carried in the decision's `writes`), so umem() works.
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

    def recall_window(self, n: int = 4, prefix: str = "conversation:") -> str:
        # Fetch the last-n transcript entries on demand (mem.recent) so the USER
        # chooses how much context to pull — apxm caps nothing. Returns the
        # entries joined oldest-first; "" if none or the host is unavailable.
        if not self._req_id:
            return ""
        items = self._host_call(HOST_METHOD_MEM_RECENT, {"prefix": prefix, "n": n})
        if not items:
            return ""
        return "\n".join(str(t) for t in items)

    def umem(self, key: str, value: Any) -> None:
        # Accumulate a memory write; the runtime applies it after the hook
        # returns (carried back in the decision's `writes` list).
        self._writes.append({"key": key, "value": value})
        return None

    def _host_call(self, method: str, params: dict[str, Any]) -> Any:
        """Call back into the runtime from inside a hook (the bridge becomes
        bidirectional for the duration). The SAME ExecutionContext that owns this
        hook services the call, so it runs under the session's backend, budget,
        and cancellation. Blocks the hook thread until the runtime replies."""
        if not self._req_id:
            raise RuntimeError(f"ctx host call '{method}' unavailable: no parent request bound")
        cb_id = f"h-{next(_host_call_counter)}"
        q: "queue.Queue[tuple[bool, Any, str | None]]" = queue.Queue(maxsize=1)
        with _host_pending_lock:
            _host_pending[cb_id] = q
        try:
            _emit_line(
                {
                    WIRE_FIELD_VERSION: WIRE_VERSION,
                    WIRE_FIELD_TYPE: WIRE_TYPE_HOST_CALL,
                    WIRE_FIELD_REQUEST_ID: cb_id,
                    WIRE_FIELD_PARENT_REQUEST_ID: self._req_id,
                    WIRE_FIELD_METHOD: method,
                    WIRE_FIELD_PARAMS: params,
                }
            )
            try:
                ok, value, error = q.get(timeout=_HOST_CALL_TIMEOUT_S)
            except queue.Empty as exc:
                raise RuntimeError(f"ctx host call '{method}' timed out") from exc
            if not ok:
                raise RuntimeError(f"ctx host call '{method}' failed: {error or 'unknown error'}")
            return value
        finally:
            with _host_pending_lock:
                _host_pending.pop(cb_id, None)

    # ---- apxm primitives the hook receives; the USER decides what to do ----

    def ask(self, prompt: str, system: str | None = None) -> str:
        """Call the runtime LLM. The user writes the prompt — apxm bakes no
        policy. Use this to summarize, classify, route, score, etc."""
        params: dict[str, Any] = {"prompt": prompt}
        if system is not None:
            params["system"] = system
        value = self._host_call(HOST_METHOD_LLM_ASK, params)
        return value if isinstance(value, str) else str(value)

    def call(self, name: str, **args: Any) -> Any:
        """Invoke any read-only apxm capability (e.g. count_tokens, http_get,
        search_skills) and get its result. apxm hands the user its tools."""
        return self._host_call(HOST_METHOD_TOOL_CALL, {"name": name, "args": args})

    def count_tokens(self, text: str) -> int:
        """Estimate the token size of *text* via the count_tokens tool, so the
        user's hook can decide on a real measure rather than a char proxy."""
        try:
            return int(self.call("count_tokens", text=text))
        except (TypeError, ValueError):
            return 0

    def recall(self, key: str) -> Any:
        """Read a session-memory key the hook itself chose (e.g. its own rolling
        summary). Returns None if unset. Pairs with ctx.umem(key, value)."""
        return self._host_call(HOST_METHOD_MEM_READ, {"key": key})


def _invoke_hook(fn: Any, event: str, payload: dict[str, Any], req_id: str = "") -> Any:
    ctx = _HookCtx(payload, req_id)
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
        ret = await loop.run_in_executor(
            None, lambda: _invoke_hook(fn, event, payload, req_id)
        )
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
            # No `shield`: the deadline is a RESOURCE limit, so a timed-out
            # (or externally cancelled) handler must actually be cancelled and
            # its compute reclaimed. `shield` would let the coroutine run to
            # completion in the background while we report a timeout — the
            # deadline would observe but not enforce. `wait_for` cancels `coro`
            # on timeout; the `except` arms below still emit a structured
            # result so cancellation never silently drops the request.
            value = await asyncio.wait_for(coro, timeout=timeout)
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

        elif msg_type == WIRE_TYPE_HOST_RESULT:
            # A reply to a host_call a hook raised; unblock the waiting thread.
            _deliver_host_result(msg)

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
