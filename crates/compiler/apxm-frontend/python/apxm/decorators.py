from __future__ import annotations

import inspect
import re
from typing import TYPE_CHECKING, Any, Callable

from .execution import CompiledFlow, ExecutionMode
from .ir import Parameter
from .proxy import GraphRecorder

if TYPE_CHECKING:
    from ._generated.models import ModelId


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
        compile_kwargs: dict[str, Any],
    ) -> None:
        self._fn = fn
        self._default_model = default_model
        self._default_system_prompt = default_system_prompt
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

    async def __call__(self, *args: Any, **kwargs: Any) -> Any:
        runtime_args = self._normalize_runtime_args(*args, **kwargs)
        return await self._compiled_flow.run(*runtime_args)

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

        # Create placeholders: support both named {topic} and positional {0} forms
        named_placeholders = {name: f"{{{name}}}" for name in self._param_mapping.keys()}
        positional_placeholders = [f"{{{idx}}}" for idx in range(len(self._param_mapping))]

        # Call the function with named placeholders
        # The function can use either {topic} or {0} style
        self._fn(recorder, **named_placeholders)

        # Post-process the graph to convert named placeholders to positional
        graph = recorder.to_graph()
        self._convert_named_to_positional(graph)

        # Stamp per-graph defaults onto LLM nodes that don't already have them
        if self._default_model is not None or self._default_system_prompt is not None:
            from .constants import LLM_OPS
            from ._generated import constants as gen_keys
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

        return graph

    def _convert_named_to_positional(self, graph: Any) -> None:
        """Convert named placeholders like {topic} to positional {0}."""
        for node in graph.nodes:
            for attr_key, attr_value in node.attributes.items():
                if isinstance(attr_value, str):
                    # Replace {param_name} with {index}
                    new_value = attr_value
                    for param_name, (idx, _) in self._param_mapping.items():
                        pattern = r'\{' + re.escape(param_name) + r'\}'
                        new_value = re.sub(pattern, f'{{{idx}}}', new_value)
                    if new_value != attr_value:
                        node.attributes[attr_key] = new_value

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
    **compile_kwargs: Any,
) -> Callable[[Callable[..., Any]], _CompiledFunction]:
    def decorator(fn: Callable[..., Any]) -> _CompiledFunction:
        return _CompiledFunction(
            fn,
            opt_level=opt_level,
            mode=mode,
            default_model=default_model,
            default_system_prompt=default_system_prompt,
            compile_kwargs=compile_kwargs,
        )

    return decorator
