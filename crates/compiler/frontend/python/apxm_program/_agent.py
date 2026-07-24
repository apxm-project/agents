"""The Agent definition marker and its compiled program handle.

``@Agent`` binds one typed program from an ``async def`` and captures its source
into the FrontendGraph on definition. The returned handle exposes ``.new(...)``
and ``.invoke(...)`` composition and the compiled graph for the compiler bridge.
"""

from __future__ import annotations

from typing import Any, Callable, Optional

from . import _bridge
from ._capture import capture_program
from ._emit import emit_frontend_graph
from ._markers import ContextSchema


class AgentDefinition:
    """A compiled typed Agent program and its composition surface."""

    def __init__(
        self,
        program_id: str,
        graph: dict[str, Any],
        input_type_ref: str,
        output_type_ref: str,
        context_type_ref: Optional[str],
    ) -> None:
        self._program_id = program_id
        self._graph = graph
        self._input_type_ref = input_type_ref
        self._output_type_ref = output_type_ref
        self._context_type_ref = context_type_ref

    @property
    def program_id(self) -> str:
        return self._program_id

    def frontend_graph(self) -> dict[str, Any]:
        """The captured FrontendGraph for this program."""
        return self._graph

    def diagnostics(self) -> Optional[str]:
        """Verification diagnostics for the captured graph, or ``None``."""
        return _bridge.verify_graph(self._graph)

    def canonical_air(self) -> str:
        """Canonical AIR JSON lowered from the captured graph."""
        return _bridge.canonical_air_json(self._graph)

    def artifact(self) -> dict[str, Any]:
        """One complete executable artifact compiled from the captured graph."""
        return _bridge.compile_artifact(self._graph)

    def new(self, *, context: Any = None) -> "ProgramInstance":
        """Create a stateful instance handle for later invocation."""
        return ProgramInstance(self._program_id, self)

    def invoke(self, _input: Any = None) -> Any:  # pragma: no cover
        raise RuntimeError("Agent.invoke is called inside a compiled Agent body")


class ProgramInstance:
    """An inferred stateful instance handle produced by ``Agent.new``."""

    def __init__(self, program_ref: str, definition: AgentDefinition) -> None:
        self.program_ref = program_ref
        self._definition = definition

    def invoke(self, _input: Any = None) -> Any:  # pragma: no cover
        raise RuntimeError("instance.invoke is called inside a compiled Agent body")


def Agent(
    *,
    input: Any,
    output: Any,
    context: Any = None,
) -> Callable[[Callable[..., Any]], AgentDefinition]:
    """Declare one typed Agent program over an ``async def`` callback."""
    input_type_ref = _type_ref(input)
    output_type_ref = _type_ref(output)
    context_type_ref: Optional[str] = None
    has_default_context = False
    if isinstance(context, ContextSchema):
        context_type_ref = context.type_ref
        has_default_context = context.default_present
    elif context is not None:
        context_type_ref = _type_ref(context)

    def decorate(func: Callable[..., Any]) -> AgentDefinition:
        bindings = _resolve_bindings(func)
        program = capture_program(
            func,
            program_id=func.__name__,
            input_type_ref=input_type_ref,
            output_type_ref=output_type_ref,
            context_type_ref=context_type_ref,
            has_default_context=has_default_context,
            bindings=bindings,
        )
        graph = emit_frontend_graph(program)
        return AgentDefinition(
            func.__name__,
            graph,
            input_type_ref,
            output_type_ref,
            context_type_ref,
        )

    return decorate


def _type_ref(annotation: Any) -> str:
    if annotation is None:
        return "None"
    if isinstance(annotation, str):
        return annotation
    if isinstance(annotation, ContextSchema):
        return annotation.type_ref
    return getattr(annotation, "__name__", str(annotation))


def _resolve_bindings(func: Callable[..., Any]) -> dict[str, Any]:
    """Collect the marker declarations visible to the callback by name."""
    namespace = dict(getattr(func, "__globals__", {}))
    closure_names = getattr(func.__code__, "co_freevars", ())
    if func.__closure__:
        for name, cell in zip(closure_names, func.__closure__):
            try:
                namespace[name] = cell.cell_contents
            except ValueError:
                continue
    return namespace
