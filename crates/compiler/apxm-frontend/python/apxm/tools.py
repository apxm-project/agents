"""@apxm.tool decorator — capture Python functions as APXM tool definitions.

Decorates a Python function so that its signature, docstring, and metadata
are captured into a FunctionTool object suitable for graph lowering and
runtime dispatch via the NDJSON tool worker.
"""

from __future__ import annotations

import hashlib
import inspect
import json
import re
from dataclasses import dataclass, field
from typing import Any, Callable, TypeVar, overload

F = TypeVar("F", bound=Callable[..., Any])

# ---------------------------------------------------------------------------
# Module-level registry — the tool worker uses this to look up handlers.
# ---------------------------------------------------------------------------
_TOOL_REGISTRY: dict[str, Callable[..., Any]] = {}

# ---------------------------------------------------------------------------
# Python type -> JSON Schema type mapping
# ---------------------------------------------------------------------------
_PY_TO_JSON_SCHEMA: dict[type, str] = {
    str: "string",
    int: "integer",
    float: "number",
    bool: "boolean",
}


# ---------------------------------------------------------------------------
# ToolContext — injected as the first argument at dispatch time
# ---------------------------------------------------------------------------
@dataclass
class ToolContext:
    """Runtime context injected into tool functions that request it.

    # Attributes
        run_id: Identifier for the current execution run.
        agent_id: Identifier for the calling agent (may be empty).
    """

    run_id: str = ""
    agent_id: str = ""


# ---------------------------------------------------------------------------
# FunctionTool — the decorated result
# ---------------------------------------------------------------------------
@dataclass
class FunctionTool:
    """Metadata wrapper produced by the @tool decorator.

    # Attributes
        name: Tool name (defaults to function name).
        description: Human-readable description from the docstring summary.
        schema_json: JSON Schema string for the tool's parameters.
        handler_id: Stable content hash (sha256 of module:qualname).
        fn: The original callable.
        metadata: Extra decorator kwargs (timeout, retries, failure_behavior).
    """

    name: str
    description: str
    schema_json: str
    handler_id: str
    fn: Callable[..., Any]
    metadata: dict[str, Any] = field(default_factory=dict)

    def __call__(self, *args: Any, **kwargs: Any) -> Any:
        return self.fn(*args, **kwargs)

    @property
    def schema(self) -> dict[str, Any]:
        return json.loads(self.schema_json)


# ---------------------------------------------------------------------------
# Docstring parsing (stdlib only — no griffe/docstring_parser dependency)
# ---------------------------------------------------------------------------
_GOOGLE_ARGS_RE = re.compile(
    r"^[ \t]*Args?:\s*$", re.MULTILINE
)
_GOOGLE_PARAM_RE = re.compile(
    r"^[ \t]+(\w+)\s*(?:\([^)]*\))?\s*:\s*(.+)", re.MULTILINE
)
_NUMPY_PARAMS_RE = re.compile(
    r"^[ \t]*Parameters?\s*$\n[ \t]*-{3,}", re.MULTILINE
)
_SPHINX_PARAM_RE = re.compile(
    r"^[ \t]*:param\s+(\w+)\s*:\s*(.+)", re.MULTILINE
)


def _parse_docstring(doc: str | None) -> tuple[str, dict[str, str]]:
    """Extract summary line and per-parameter descriptions from a docstring.

    Supports Google, NumPy, and Sphinx styles (best-effort, no external deps).

    # Returns
        (summary, {param_name: description})
    """
    if not doc:
        return "", {}

    lines = doc.strip().splitlines()
    summary = lines[0].strip() if lines else ""

    param_descs: dict[str, str] = {}

    # Try Sphinx first (:param name: ...)
    for m in _SPHINX_PARAM_RE.finditer(doc):
        param_descs[m.group(1)] = m.group(2).strip()

    if param_descs:
        return summary, param_descs

    # Try Google (Args: / Arguments:)
    args_match = _GOOGLE_ARGS_RE.search(doc)
    if args_match:
        after_args = doc[args_match.end():]
        # Collect param lines until we hit a blank line or un-indented section
        for m in _GOOGLE_PARAM_RE.finditer(after_args):
            param_descs[m.group(1)] = m.group(2).strip()
            # Stop at the next section header (un-indented non-blank line that
            # doesn't look like a param)
        if param_descs:
            return summary, param_descs

    # Try NumPy (Parameters\n----------)
    numpy_match = _NUMPY_PARAMS_RE.search(doc)
    if numpy_match:
        after_header = doc[numpy_match.end():]
        for m in re.finditer(r"^\s*(\w+)\s*:.*$\n\s+(.+)", after_header, re.MULTILINE):
            param_descs[m.group(1)] = m.group(2).strip()

    return summary, param_descs


# ---------------------------------------------------------------------------
# Schema generation from inspect.signature
# ---------------------------------------------------------------------------

def _build_schema(
    fn: Callable[..., Any],
    *,
    strict: bool = True,
    wants_context: bool = False,
) -> str:
    """Build a JSON Schema string from a function's signature.

    # Arguments
        fn: The function to introspect.
        strict: If True, set additionalProperties to false.
        wants_context: If True, the first parameter is ToolContext and is
                       excluded from the schema.

    # Returns
        JSON string of the parameter schema.
    """
    sig = inspect.signature(fn)
    params = list(sig.parameters.values())

    # Strip ToolContext first-arg
    if wants_context and params:
        params = params[1:]

    _, param_descs = _parse_docstring(fn.__doc__)

    properties: dict[str, Any] = {}
    required: list[str] = []

    for p in params:
        prop: dict[str, Any] = {}

        # Map annotation to JSON Schema type
        ann = p.annotation
        if ann is inspect.Parameter.empty:
            prop["type"] = "string"
        elif ann in _PY_TO_JSON_SCHEMA:
            prop["type"] = _PY_TO_JSON_SCHEMA[ann]
        elif ann is list:
            prop["type"] = "array"
        elif ann is dict:
            prop["type"] = "object"
        else:
            # Fall back to string for unknown types
            prop["type"] = "string"

        # Add description from docstring
        desc = param_descs.get(p.name)
        if desc:
            prop["description"] = desc

        properties[p.name] = prop

        # Required unless it has a default
        if p.default is inspect.Parameter.empty:
            required.append(p.name)

    schema: dict[str, Any] = {
        "type": "object",
        "properties": properties,
    }
    if required:
        schema["required"] = required
    if strict:
        schema["additionalProperties"] = False

    return json.dumps(schema, separators=(",", ":"))


# ---------------------------------------------------------------------------
# handler_id generation
# ---------------------------------------------------------------------------

def _make_handler_id(fn: Callable[..., Any]) -> str:
    """Compute a stable handler_id from module:qualname."""
    module = getattr(fn, "__module__", "__unknown__") or "__unknown__"
    qualname = getattr(fn, "__qualname__", fn.__name__)
    key = f"{module}:{qualname}"
    return f"sha256:{hashlib.sha256(key.encode('utf-8')).hexdigest()}"


# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------

def _validate_tool_function(fn: Callable[..., Any]) -> bool:
    """Check that fn is a valid tool function. Returns True if first arg is ToolContext.

    Raises TypeError for async functions or functions with *args/**kwargs.
    """
    if inspect.iscoroutinefunction(fn):
        raise TypeError(
            f"@tool does not support async functions (got {fn.__qualname__}). "
            "Use a sync function."
        )

    sig = inspect.signature(fn)
    params = list(sig.parameters.values())

    for p in params:
        if p.kind == inspect.Parameter.VAR_POSITIONAL:
            raise TypeError(
                f"@tool does not support *args (in {fn.__qualname__}). "
                "Declare each parameter explicitly."
            )
        if p.kind == inspect.Parameter.VAR_KEYWORD:
            raise TypeError(
                f"@tool does not support **kwargs (in {fn.__qualname__}). "
                "Declare each parameter explicitly."
            )

    # Detect ToolContext first-arg
    wants_context = False
    if params:
        first = params[0]
        ann = first.annotation
        if ann is ToolContext:
            wants_context = True
        elif isinstance(ann, str) and ann in ("ToolContext", "apxm.ToolContext"):
            wants_context = True

    return wants_context


# ---------------------------------------------------------------------------
# @tool decorator
# ---------------------------------------------------------------------------

@overload
def tool(fn: F, /) -> FunctionTool: ...

@overload
def tool(
    *,
    name: str | None = None,
    timeout: int | None = None,
    strict: bool = True,
    retries: int = 0,
    failure_behavior: str = "raise",
) -> Callable[[F], FunctionTool]: ...


def tool(
    fn: F | None = None,
    /,
    *,
    name: str | None = None,
    timeout: int | None = None,
    strict: bool = True,
    retries: int = 0,
    failure_behavior: str = "raise",
) -> FunctionTool | Callable[[F], FunctionTool]:
    """Decorate a Python function as an APXM tool.

    Can be used bare (``@tool``) or with arguments (``@tool(timeout=5000)``).

    # Arguments
        fn: The function (when used as ``@tool`` without parens).
        name: Override the tool name (defaults to ``fn.__name__``).
        timeout: Per-call timeout in milliseconds.
        strict: If True (default), schema sets ``additionalProperties: false``.
        retries: Number of retry attempts on failure (default 0).
        failure_behavior: What to do on failure — ``"raise"`` (default) or
                          ``"return_error"``.
    """

    def _wrap(f: F) -> FunctionTool:
        wants_context = _validate_tool_function(f)

        tool_name = name if name is not None else f.__name__
        summary, _ = _parse_docstring(f.__doc__)
        handler_id = _make_handler_id(f)
        schema_json = _build_schema(f, strict=strict, wants_context=wants_context)

        metadata: dict[str, Any] = {}
        if timeout is not None:
            metadata["timeout"] = timeout
        if retries:
            metadata["retries"] = retries
        if failure_behavior != "raise":
            metadata["failure_behavior"] = failure_behavior
        metadata["wants_context"] = wants_context

        ft = FunctionTool(
            name=tool_name,
            description=summary,
            schema_json=schema_json,
            handler_id=handler_id,
            fn=f,
            metadata=metadata,
        )

        # Register globally for worker lookup
        _TOOL_REGISTRY[handler_id] = f

        return ft

    if fn is not None:
        return _wrap(fn)
    return _wrap
