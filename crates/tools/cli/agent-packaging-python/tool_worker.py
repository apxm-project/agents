"""Executes bundled Python Capability handlers for the composition root.

The sibling of `tool-worker.mjs`, speaking the identical frame protocol: one
JSON object per line in, one result frame per line out, correlated by `req_id`.
Arguments arrive as the single mapping the declaration's schema describes, a
result is the `answer(...)` envelope's value, and any raised exception becomes
`ok: false` with its message rather than a dead stream — the same three cases
the Node worker reports, so the Rust side reads one reply shape.

The worker is handed a materialized manifest and evaluates those bytes. It never
reads the package it came from.
"""

from __future__ import annotations

import importlib.util
import json
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Any, Callable

from apxm_program.handlers import ANSWER_KIND, HANDLER_ID_PREFIX, CapabilityId

sys.dont_write_bytecode = True


def load_handlers(manifest_path: Path, directory: Path) -> dict[str, Callable[..., Any]]:
    """Materialize every descriptor's source and bind it to its handler id."""
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    handlers: dict[str, Callable[..., Any]] = {}
    for entry in manifest.get("handlers", []):
        handler_id = entry["handler_id"]
        source = directory / f"{handler_id[len(HANDLER_ID_PREFIX):]}.py"
        source.write_text(entry["source"]["content"], encoding="utf-8")
        # Loaded under the module id the identity was hashed from, so the
        # declaration recomputes the same handler_id the manifest carries.
        spec = importlib.util.spec_from_file_location(entry["module"], source)
        if spec is None or spec.loader is None:
            raise SystemExit(f"bundled handler {handler_id} is not loadable")
        module = importlib.util.module_from_spec(spec)
        sys.modules[entry["module"]] = module
        spec.loader.exec_module(module)
        declared = next(
            (
                value
                for value in vars(module).values()
                if isinstance(value, CapabilityId)
                and value.descriptor.handler_id == handler_id
            ),
            None,
        )
        if declared is None:
            raise SystemExit(f"bundled handler {handler_id} is missing")
        handlers[handler_id] = declared.descriptor.run
    return handlers


def reply(req_id: Any, **fields: Any) -> None:
    print(json.dumps({"v": 1, "type": "result", "req_id": req_id, **fields}), flush=True)


def main() -> None:
    if len(sys.argv) < 2:
        raise SystemExit("handler manifest path is required")
    directory = Path(tempfile.mkdtemp(prefix="apxm-handler-worker-"))
    try:
        handlers = load_handlers(Path(sys.argv[1]), directory)
        for line in sys.stdin:
            if not line.strip():
                continue
            frame = json.loads(line)
            handler = handlers.get(frame.get("tool_id"))
            if handler is None:
                reply(frame.get("req_id"), ok=False, error=f"unknown handler {frame.get('tool_id')}")
                continue
            try:
                answer = handler(frame.get("args") or {})
                if not isinstance(answer, dict) or answer.get("kind") != ANSWER_KIND:
                    raise TypeError("a packaged Tool handler must return answer({...})")
                reply(frame["req_id"], ok=True, value=answer["value"])
            except Exception as error:  # noqa: BLE001 - reported, never swallowed
                reply(frame["req_id"], ok=False, error=str(error) or type(error).__name__)
    finally:
        shutil.rmtree(directory, ignore_errors=True)


if __name__ == "__main__":
    main()
