"""Bundles Python Capability handlers into the portable handler manifest.

The sibling of `compile-handlers.mjs`, and deliberately the same shape: it loads
each handler source under its package-relative module id, reads what the
declaration states about itself, and emits one `apxm.handler-manifest`
descriptor per shipped Capability.

Where the two differ is what "bundle" means. esbuild must inline
`@apxm/agent-packaging` because a Node worker importing from a temp directory
cannot resolve it. A Python worker has `apxm_program` importable already, so the
artifact-local source is the handler module's own text and its imports resolve
from the installed environment at load. That makes a Python package handler one
module: a handler that imports a package-local sibling is bundled without it and
fails at load, where the missing import is visible.
"""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from typing import Any

from apxm_program.handlers import HANDLER_ID_PREFIX, CapabilityId

# Loading a handler out of a package directory must not leave build litter in it.
sys.dont_write_bytecode = True

MANIFEST_VERSION = "apxm.handler-manifest"
SOURCE_DIRECTORY = "handlers"
LANGUAGE = "python"


def module_id_from_path(entry: Path, root: Path) -> str:
    """The package-relative module id a descriptor's `handler_id` hashes."""
    try:
        relative = entry.resolve().relative_to(root.resolve())
    except ValueError:
        raise SystemExit(f"handler {entry} is outside package root {root}")
    return relative.with_suffix("").as_posix()


def load_module(source: Path, module_id: str) -> Any:
    """Load one handler source under the exact module id its identity uses."""
    spec = importlib.util.spec_from_file_location(module_id, source)
    if spec is None or spec.loader is None:
        raise SystemExit(f"handler {source} is not loadable as a Python module")
    module = importlib.util.module_from_spec(spec)
    # Registered before execution so a dataclass or pickle lookup inside the
    # handler body resolves the module it is being defined in.
    sys.modules[module_id] = module
    spec.loader.exec_module(module)
    return module


def compile_handlers(entries: list[Path], root: Path) -> dict[str, Any]:
    """Compile package-local Python handlers into the canonical sidecar."""
    handlers = []
    seen = set()
    for entry in entries:
        module_id = module_id_from_path(entry, root)
        module = load_module(entry, module_id)
        content = entry.read_text(encoding="utf-8")
        for name in sorted(vars(module)):
            declared = getattr(module, name)
            if not isinstance(declared, CapabilityId):
                continue
            descriptor = declared.descriptor
            if descriptor.handler_id in seen:
                continue
            seen.add(descriptor.handler_id)
            digest = descriptor.handler_id[len(HANDLER_ID_PREFIX) :]
            handlers.append(
                {
                    **descriptor.manifest_entry(),
                    "language": LANGUAGE,
                    "source": {
                        "artifact_path": f"{SOURCE_DIRECTORY}/{digest}.py",
                        "content": content,
                    },
                }
            )
    return {"version": MANIFEST_VERSION, "handlers": handlers}


def main() -> None:
    out, root, *sources = sys.argv[1:]
    manifest = compile_handlers([Path(s) for s in sources], Path(root))
    Path(out).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
