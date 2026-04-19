from __future__ import annotations

import inspect
from typing import TYPE_CHECKING, Any, Callable

from .execution import CompiledFlow, ExecutionMode
from .ir import Parameter
from .proxy import GraphRecorder

if TYPE_CHECKING:
    from ._generated.models import ModelId
    from ._generated.providers import ProviderSpec


try:
    from ._generated.constants import VALID_PARAM_TYPES as _VALID_TYPES
except ImportError:
    _VALID_TYPES = frozenset({"str", "int", "float", "bool", "json"})

_PYTHON_TYPE_TO_APXM: dict[type | str, str] = {
    str: "str",
    int: "int",
    float: "float",
    bool: "bool",
    dict: "json",
    list: "json",
    "str": "str",
    "int": "int",
    "float": "float",
    "bool": "bool",
    "dict": "json",
    "list": "json",
    "Any": "json",
}


class _CompiledFunction:
    def __init__(
        self,
        fn: Callable[..., Any],
        *,
        opt_level: int,
        mode: ExecutionMode,
        default_model: ModelId | None = None,
        default_system_prompt: str | None = None,
        default_provider: ProviderSpec | None = None,
        default_backend: str | None = None,
        compile_kwargs: dict[str, Any],
    ) -> None:
        self._fn = fn
        self._default_model = default_model
        self._default_system_prompt = default_system_prompt
        self._default_provider = default_provider
        self._default_backend = default_backend
        self._signature = inspect.signature(fn)
        self._param_mapping = self._derive_parameters()
        self._graph = self._capture_graph()
        self._compiled_flow = CompiledFlow(
            self._graph,
            mode=mode,
            opt_level=opt_level,
            **compile_kwargs,
        )
        self.__name__ = fn.__name__
        self.__doc__ = fn.__doc__

    async def __call__(self, *args: Any, session_id: str | None = None, **kwargs: Any) -> Any:
        runtime_args = self._normalize_runtime_args(*args, **kwargs)
        return await self._compiled_flow.run(*runtime_args, session_id=session_id)

    def run_sync(self, *args: Any, session_id: str | None = None, **kwargs: Any) -> Any:
        """Synchronous execution convenience method."""
        runtime_args = self._normalize_runtime_args(*args, **kwargs)
        return self._compiled_flow.run_sync(*runtime_args, session_id=session_id)

    async def stream(self, *args: Any, session_id: str | None = None, **kwargs: Any):
        """Async generator yielding execution events via SSE."""
        runtime_args = self._normalize_runtime_args(*args, **kwargs)
        async for event in self._compiled_flow.stream(*runtime_args, session_id=session_id):
            yield event

    def _derive_parameters(self) -> dict[str, tuple[int, str]]:
        """Derive graph parameters from function signature.

        Returns a mapping of parameter name -> (index, type_name).
        """
        params = list(self._signature.parameters.values())
        if not params:
            raise ValueError("decorated function must accept a GraphRecorder as the first argument")

        param_mapping: dict[str, tuple[int, str]] = {}

        # Skip the first parameter (GraphRecorder)
        for idx, param in enumerate(params[1:]):
            # Infer type from annotation
            type_name = "str"  # default
            if param.annotation != inspect.Parameter.empty:
                annotation = param.annotation
                # Handle string annotations (from __future__ import annotations)
                if isinstance(annotation, str):
                    type_name = _PYTHON_TYPE_TO_APXM.get(annotation, "str")
                else:
                    type_name = _PYTHON_TYPE_TO_APXM.get(annotation, "str")

            param_mapping[param.name] = (idx, type_name)

        return param_mapping

    def _capture_graph(self):
        params = list(self._signature.parameters.values())
        if not params:
            raise ValueError("decorated function must accept a GraphRecorder as the first argument")

        recorder = GraphRecorder(self._fn.__name__)

        # Add parameters to the graph based on function signature
        for param_name, (idx, type_name) in self._param_mapping.items():
            recorder.param(param_name, type_name)

        # Pass named placeholders into the user function so f-strings like
        # f"Process {topic}" yield the literal string "Process {topic}". The
        # compiler validator resolves each `{name}` against either the node's
        # input_names attribute or the module's declared parameters.
        named_placeholders = {name: f"{{{name}}}" for name in self._param_mapping.keys()}
        self._fn(recorder, **named_placeholders)

        graph = recorder.to_graph()

        # Stamp per-graph defaults onto LLM nodes that don't already have them
        if (
            self._default_model is not None
            or self._default_system_prompt is not None
            or self._default_provider is not None
            or self._default_backend is not None
        ):
            from .constants import LLM_OPS
            from ._generated import constants as gen_keys
            from .normalize import normalize_provider as _normalize_provider
            from .normalize import normalize_value as _normalize_value

            if self._default_model is not None:
                model_str = _normalize_value(self._default_model)
                for node in graph.nodes:
                    if node.op in LLM_OPS and gen_keys.MODEL not in node.attributes:
                        node.attributes[gen_keys.MODEL] = model_str

            if self._default_system_prompt is not None:
                for node in graph.nodes:
                    if node.op in LLM_OPS and gen_keys.SYSTEM_PROMPT not in node.attributes:
                        node.attributes[gen_keys.SYSTEM_PROMPT] = self._default_system_prompt

            if self._default_provider is not None:
                provider_value = _normalize_provider(self._default_provider)
                for node in graph.nodes:
                    if node.op in LLM_OPS and gen_keys.PROVIDER not in node.attributes:
                        node.attributes[gen_keys.PROVIDER] = provider_value

            if self._default_backend is not None:
                for node in graph.nodes:
                    if node.op in LLM_OPS and gen_keys.BACKEND not in node.attributes:
                        node.attributes[gen_keys.BACKEND] = self._default_backend

        return graph

    def _normalize_runtime_args(self, *args: Any, **kwargs: Any) -> tuple[Any, ...]:
        # Build arg list in parameter order
        if kwargs:
            bound = self._signature.bind(None, *args, **kwargs)
            bound.apply_defaults()
            arg_values = []
            for param_name in self._param_mapping.keys():
                if param_name in bound.arguments:
                    arg_values.append(bound.arguments[param_name])
                else:
                    raise TypeError(f"missing required argument: '{param_name}'")
            return tuple(arg_values)
        return args


def compile(
    *,
    opt_level: int = 2,
    mode: ExecutionMode = ExecutionMode.COMPILED,
    default_model: ModelId | None = None,
    default_system_prompt: str | None = None,
    default_provider: ProviderSpec | None = None,
    default_backend: str | None = None,
    **compile_kwargs: Any,
) -> Callable[[Callable[..., Any]], _CompiledFunction]:
    def decorator(fn: Callable[..., Any]) -> _CompiledFunction:
        return _CompiledFunction(
            fn,
            opt_level=opt_level,
            mode=mode,
            default_model=default_model,
            default_system_prompt=default_system_prompt,
            default_provider=default_provider,
            default_backend=default_backend,
            compile_kwargs=compile_kwargs,
        )

    return decorator
