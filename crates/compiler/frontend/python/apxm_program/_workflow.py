"""Typed compiled Workflow handle shared by Agent authoring and composition."""

from __future__ import annotations

import hashlib
import json
from typing import Any, Awaitable, Callable, Generic, Optional, Protocol, TypeVar, overload

from . import _bridge, _native
from ._markers import ContextSchema

InputT = TypeVar("InputT")
OutputT = TypeVar("OutputT")
ContextT = TypeVar("ContextT")


class Program(Protocol[InputT, OutputT, ContextT]):
    """One compiled program, including Context, loops, Hooks and yield/resume."""

    @property
    def program_id(self) -> str: ...

    @property
    def _program_reference(self) -> tuple[str, str, str, str]: ...

    def frontend_graph(self) -> dict[str, Any]: ...
    def diagnostics(self) -> Optional[str]: ...
    def canonical_air(self) -> str: ...
    def artifact(self) -> dict[str, Any]: ...
    def new(self, *, context: Optional[ContextT] = None) -> ProgramInstance[InputT, OutputT, ContextT]: ...
    def invoke(self, _input: InputT) -> Awaitable[OutputT]: ...


@overload
def Workflow(*, input: type[InputT], output: type[OutputT], context: None = None) -> Callable[[Callable[..., Awaitable[OutputT]]], Program[InputT, OutputT, None]]: ...

@overload
def Workflow(*, input: type[InputT], output: type[OutputT], context: type[ContextT] | ContextSchema[ContextT]) -> Callable[[Callable[..., Awaitable[OutputT]]], Program[InputT, OutputT, ContextT]]: ...


def Workflow(
    *, input: type[InputT], output: type[OutputT],
    context: type[ContextT] | ContextSchema[ContextT] | None = None,
) -> Callable[[Callable[..., Awaitable[OutputT]]], Program[InputT, OutputT, ContextT]]:
    """General orchestration; Model calls are optional and never implicit."""
    from ._agent import _define_program

    return _define_program(input=input, output=output, context=context)


class _FrozenDefinitionType(type):
    """Prevent evaluated source from replacing Agent handle methods."""

    def __setattr__(cls, name: str, value: Any) -> None:
        if cls.__dict__.get("_methods_frozen", False):
            raise AttributeError("Workflow methods are immutable after import")
        super().__setattr__(name, value)


class ProgramHandle(Generic[InputT, OutputT, ContextT], metaclass=_FrozenDefinitionType):
    """A compiled typed Agent program and its composition surface."""

    __slots__ = (
        "_program_id",
        "_input_type_ref",
        "_output_type_ref",
        "_context_type_ref",
        "_artifact_digest",
        "_sealed",
        "__weakref__",
    )

    def __setattr__(self, name: str, value: Any) -> None:
        """Keep the captured handle's methods and snapshot reference stable."""
        if getattr(self, "_sealed", False):
            raise AttributeError("ProgramHandle is immutable after capture")
        object.__setattr__(self, name, value)

    def __init__(
        self,
        program_id: str,
        graph: dict[str, Any],
        input_type_ref: str,
        output_type_ref: str,
        context_type_ref: Optional[str],
    ) -> None:
        self._program_id = program_id
        # Keep the canonical snapshot in the native bridge. Python objects,
        # function defaults, and module globals are reflective from evaluated
        # source and therefore cannot serve as an integrity boundary.
        graph_json = json.dumps(graph, sort_keys=True, separators=(",", ":"))
        self._input_type_ref = input_type_ref
        self._output_type_ref = output_type_ref
        self._context_type_ref = context_type_ref
        self._artifact_digest = "sha256:" + hashlib.sha256(
            graph_json.encode("utf-8")
        ).hexdigest()
        _native.seal_frontend_graph(self, graph_json)
        self._sealed = True

    def _graph_value(self) -> dict[str, Any]:
        """Decode an independent graph value from the immutable snapshot."""
        return _native.frontend_graph(self)

    @property
    def program_id(self) -> str:
        return self._program_id

    @property
    def _program_reference(self) -> tuple[str, str, str, str]:
        """Return the static composition reference consumed by source capture."""
        return (
            self._program_id,
            self._artifact_digest,
            self._program_id,
            f"{self._program_id}.identity",
        )

    def frontend_graph(self) -> dict[str, Any]:
        """The captured FrontendGraph for this program."""
        return _native.frontend_graph(self)

    def diagnostics(self) -> Optional[str]:
        """Verification diagnostics for the captured graph, or ``None``."""
        return _bridge.verify_graph(self._graph_value())

    def canonical_air(self) -> str:
        """Canonical AIR JSON lowered from the captured graph."""
        return _bridge.canonical_air_json(self._graph_value())

    def artifact(self) -> dict[str, Any]:
        """One complete executable artifact compiled from the captured graph."""
        return _bridge.compile_artifact(self._graph_value())

    def new(self, *, context: Optional[ContextT] = None) -> "ProgramInstance[InputT, OutputT, ContextT]":
        """Create a stateful instance handle for later invocation."""
        return ProgramInstance(self._program_id, self)

    def invoke(self, _input: InputT) -> Awaitable[OutputT]:  # pragma: no cover
        raise RuntimeError("Agent.invoke is called inside a compiled Agent body")


type.__setattr__(ProgramHandle, "_methods_frozen", True)


class ProgramInstance(Generic[InputT, OutputT, ContextT]):
    """An inferred stateful instance handle produced by ``Agent.new``."""

    def __init__(self, program_ref: str, definition: Program[InputT, OutputT, ContextT]) -> None:
        self.program_ref = program_ref
        self._definition = definition

    def invoke(self, _input: InputT) -> Awaitable[OutputT]:  # pragma: no cover
        raise RuntimeError("instance.invoke is called inside a compiled Agent body")
