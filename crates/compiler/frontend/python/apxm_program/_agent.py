"""The Agent definition marker and its compiled program handle.

``@Agent`` binds one typed program from an ``async def`` and captures its source
into the FrontendGraph on definition. The returned handle exposes ``.new(...)``
and ``.invoke(...)`` composition and the compiled graph for the compiler bridge.
"""

from __future__ import annotations

from dataclasses import replace
from typing import Any, Awaitable, Callable, Optional, overload

from ._capture import capture_program
from ._emit import emit_frontend_graph
from ._generated.diagnostics import (
    AGENT_DYNAMIC_ARGUMENT,
    AGENT_MISSING_INPUT_OUTPUT,
)
from ._markers import ContextSchema, ModelBinding
from ._input_schema import checked_input_schema
from ._generated.frontend_records import AgentAuthoring, WorkflowAuthoring
from ._workflow import ContextT, InputT, OutputT, ProgramHandle, Program


@overload
def Agent(*, input: type[InputT], output: type[OutputT], model: ModelBinding, context: None = None) -> Callable[[Callable[..., Awaitable[OutputT]]], Program[InputT, OutputT, None]]: ...

@overload
def Agent(*, input: type[InputT], output: type[OutputT], model: ModelBinding, context: type[ContextT] | ContextSchema[ContextT]) -> Callable[[Callable[..., Awaitable[OutputT]]], Program[InputT, OutputT, ContextT]]: ...


def Agent(
    *,
    input: type[InputT],
    output: type[OutputT],
    model: ModelBinding,
    context: type[ContextT] | ContextSchema[ContextT] | None = None,
) -> Callable[[Callable[..., Awaitable[OutputT]]], Program[InputT, OutputT, ContextT]]:
    """A model-backed program must invoke its explicitly bound primary Model."""
    if not isinstance(model, ModelBinding):
        raise TypeError(f"{AGENT_DYNAMIC_ARGUMENT}: Agent requires an explicit typed Model binding")
    return _define_program(input=input, output=output, context=context, model=model)


def _define_program(
    *, input: type[InputT], output: type[OutputT],
    context: type[ContextT] | ContextSchema[ContextT] | None,
    model: ModelBinding | None = None,
) -> Callable[[Callable[..., Awaitable[OutputT]]], Program[InputT, OutputT, ContextT]]:
    """The shared capture boundary behind both public program declarations."""
    input_type_ref = _type_ref(input)
    output_type_ref = _type_ref(output)
    context_type_ref: Optional[str] = None
    has_default_context = False
    if isinstance(context, ContextSchema):
        context_type_ref = context.type_ref
        has_default_context = context.default_present
    elif context is not None:
        context_type_ref = _type_ref(context)

    def decorate(func: Callable[..., Awaitable[OutputT]]) -> Program[InputT, OutputT, ContextT]:
        bindings = _resolve_bindings(func)
        program = capture_program(
            func,
            program_id=func.__name__,
            input_type_ref=input_type_ref,
            input_annotation=input,
            output_type_ref=output_type_ref,
            context_type_ref=context_type_ref,
            has_default_context=has_default_context,
            bindings=bindings,
            context_schema_type=context.schema_type if isinstance(context, ContextSchema) else context,
        )
        program = replace(
            program,
            input_schema=checked_input_schema(input, bindings),
            authoring=WorkflowAuthoring(kind="workflow") if model is None else AgentAuthoring(kind="agent", primary_model_ref=model.target_ref),
        )
        if model is not None:
            binding_names = {f"decl.model.{name}" for name, binding in bindings.items() if binding is model}
            if not any(call.contract.intent_kind == "model_invocation" and call.contract.binding_ref in binding_names for call in program.calls):
                raise TypeError(f"{AGENT_DYNAMIC_ARGUMENT}: Agent must invoke its explicitly declared primary Model binding")
        graph = emit_frontend_graph(program)
        return ProgramHandle[InputT, OutputT, ContextT](
            func.__name__,
            graph,
            input_type_ref,
            output_type_ref,
            context_type_ref,
        )

    return decorate


def _type_ref(annotation: Any) -> str:
    """Resolve one typed Agent interface declaration to its type reference.

    The declaration is the type, never a string naming it: a renamed type
    renames its reference, and a reference to a type that does not exist is a
    NameError at the definition site rather than a graph that disagrees with the
    source. TypeScript reads the same three off `Agent<Input, Output, Context>`.
    """
    if isinstance(annotation, ContextSchema):
        return annotation.type_ref
    if annotation is None:
        raise TypeError(
            f"{AGENT_MISSING_INPUT_OUTPUT}: an Agent states the typed input it "
            "takes and the typed output it returns"
        )
    if isinstance(annotation, str):
        raise TypeError(
            f"{AGENT_DYNAMIC_ARGUMENT}: an Agent input, output, and Context are the "
            f"typed declarations themselves, not the string {annotation!r} naming one"
        )
    name = getattr(annotation, "__name__", None)
    if name is None:
        raise TypeError(
            f"{AGENT_DYNAMIC_ARGUMENT}: an Agent input, output, and Context are "
            f"typed declarations; got {annotation!r}"
        )
    return name


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
