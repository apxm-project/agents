"""Tests for @apxm.tool decorator and FunctionTool."""

import json

import pytest

from apxm.tools import (
    FunctionTool,
    ToolContext,
    _TOOL_REGISTRY,
    _make_handler_id,
    _parse_docstring,
    tool,
)


# ---------------------------------------------------------------------------
# Basic decorator usage
# ---------------------------------------------------------------------------


class TestToolDecoratorBasic:
    def test_bare_decorator(self):
        """@tool without parens wraps a function into FunctionTool."""

        @tool
        def add(a: int, b: int) -> int:
            """Add two integers."""
            return a + b

        assert isinstance(add, FunctionTool)
        assert add.name == "add"
        assert add.description == "Add two integers."

    def test_decorator_with_parens(self):
        """@tool() with empty parens also works."""

        @tool()
        def sub(a: int, b: int) -> int:
            """Subtract b from a."""
            return a - b

        assert isinstance(sub, FunctionTool)
        assert sub.name == "sub"

    def test_decorator_with_kwargs(self):
        """@tool(timeout=5000, retries=2) passes metadata through."""

        @tool(timeout=5000, retries=2, failure_behavior="return_error")
        def flaky(x: str) -> str:
            """A flaky tool."""
            return x

        assert flaky.metadata["timeout"] == 5000
        assert flaky.metadata["retries"] == 2
        assert flaky.metadata["failure_behavior"] == "return_error"

    def test_custom_name(self):
        """@tool(name="my_tool") overrides the function name."""

        @tool(name="custom_name")
        def original(x: str) -> str:
            return x

        assert original.name == "custom_name"

    def test_callable(self):
        """FunctionTool is callable and delegates to the original function."""

        @tool
        def multiply(a: int, b: int) -> int:
            return a * b

        assert multiply(3, 4) == 12


# ---------------------------------------------------------------------------
# Schema generation
# ---------------------------------------------------------------------------


class TestSchemaGeneration:
    def test_basic_types(self):
        """int, str, float, bool map to JSON Schema types."""

        @tool
        def typed(a: int, b: str, c: float, d: bool) -> str:
            return ""

        schema = typed.schema
        assert schema["properties"]["a"]["type"] == "integer"
        assert schema["properties"]["b"]["type"] == "string"
        assert schema["properties"]["c"]["type"] == "number"
        assert schema["properties"]["d"]["type"] == "boolean"

    def test_required_params(self):
        """Parameters without defaults are required."""

        @tool
        def required(a: int, b: str) -> str:
            return ""

        schema = typed_schema(required)
        assert schema["required"] == ["a", "b"]

    def test_optional_params(self):
        """Parameters with defaults are not required."""

        @tool
        def optional(a: int, b: str = "hello") -> str:
            return ""

        schema = typed_schema(optional)
        assert schema["required"] == ["a"]
        assert "b" in schema["properties"]

    def test_strict_mode_default(self):
        """By default, additionalProperties is false (strict=True)."""

        @tool
        def strict_fn(a: int) -> int:
            return a

        schema = typed_schema(strict_fn)
        assert schema["additionalProperties"] is False

    def test_non_strict_mode(self):
        """strict=False omits additionalProperties."""

        @tool(strict=False)
        def loose_fn(a: int) -> int:
            return a

        schema = typed_schema(loose_fn)
        assert "additionalProperties" not in schema

    def test_unannotated_defaults_to_string(self):
        """Parameters without type annotations default to string."""

        @tool
        def untyped(x) -> str:
            return str(x)

        schema = typed_schema(untyped)
        assert schema["properties"]["x"]["type"] == "string"

    def test_list_and_dict_types(self):
        """list -> array, dict -> object."""

        @tool
        def complex_types(items: list, config: dict) -> str:
            return ""

        schema = typed_schema(complex_types)
        assert schema["properties"]["items"]["type"] == "array"
        assert schema["properties"]["config"]["type"] == "object"

    def test_no_params_produces_empty_schema(self):
        """A tool with no params has empty properties."""

        @tool
        def no_args() -> str:
            return "hello"

        schema = typed_schema(no_args)
        assert schema["properties"] == {}
        assert "required" not in schema


# ---------------------------------------------------------------------------
# ToolContext detection
# ---------------------------------------------------------------------------


class TestToolContext:
    def test_context_stripped_from_schema(self):
        """First-arg ToolContext is stripped from the schema."""

        @tool
        def ctx_tool(ctx: ToolContext, query: str) -> str:
            """Search something."""
            return query

        schema = typed_schema(ctx_tool)
        assert "ctx" not in schema["properties"]
        assert "query" in schema["properties"]
        assert schema["required"] == ["query"]

    def test_context_flag_in_metadata(self):
        """wants_context is set in metadata."""

        @tool
        def ctx_tool(ctx: ToolContext, x: int) -> int:
            return x

        assert ctx_tool.metadata["wants_context"] is True

    def test_no_context_flag(self):
        """Functions without ToolContext have wants_context=False."""

        @tool
        def plain(x: int) -> int:
            return x

        assert plain.metadata.get("wants_context", False) is False


# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------


class TestValidation:
    def test_async_rejected(self):
        """async def functions are rejected with a clear error."""
        with pytest.raises(TypeError, match="async"):

            @tool
            async def async_fn(x: int) -> int:
                return x

    def test_varargs_rejected(self):
        """*args is rejected."""
        with pytest.raises(TypeError, match="\\*args"):

            @tool
            def varargs_fn(*args: int) -> int:
                return sum(args)

    def test_kwargs_rejected(self):
        """**kwargs is rejected."""
        with pytest.raises(TypeError, match="\\*\\*kwargs"):

            @tool
            def kwargs_fn(**kwargs: str) -> str:
                return ""


# ---------------------------------------------------------------------------
# handler_id and registry
# ---------------------------------------------------------------------------


class TestHandlerId:
    def test_stable_hash(self):
        """handler_id is deterministic for the same module:qualname."""

        @tool
        def stable_fn(x: int) -> int:
            return x

        id1 = stable_fn.handler_id
        # Re-compute manually
        id2 = _make_handler_id(stable_fn.fn)
        assert id1 == id2

    def test_registered_in_global_registry(self):
        """Decorated tools are registered in _TOOL_REGISTRY."""

        @tool
        def registered_fn(x: str) -> str:
            return x

        assert registered_fn.handler_id in _TOOL_REGISTRY
        assert _TOOL_REGISTRY[registered_fn.handler_id] is registered_fn.fn


# ---------------------------------------------------------------------------
# Docstring parsing
# ---------------------------------------------------------------------------


class TestDocstringParsing:
    def test_google_style(self):
        summary, params = _parse_docstring(
            """Search the web.

            Args:
                query: The search query string.
                limit: Maximum number of results.
            """
        )
        assert summary == "Search the web."
        assert params["query"] == "The search query string."
        assert params["limit"] == "Maximum number of results."

    def test_sphinx_style(self):
        summary, params = _parse_docstring(
            """Compute a sum.

            :param a: First operand.
            :param b: Second operand.
            """
        )
        assert summary == "Compute a sum."
        assert params["a"] == "First operand."
        assert params["b"] == "Second operand."

    def test_numpy_style(self):
        summary, params = _parse_docstring(
            """Transform data.

            Parameters
            ----------
            data : array
                The input data.
            factor : float
                Scaling factor.
            """
        )
        assert summary == "Transform data."
        assert params["data"] == "The input data."
        assert params["factor"] == "Scaling factor."

    def test_no_docstring(self):
        summary, params = _parse_docstring(None)
        assert summary == ""
        assert params == {}

    def test_summary_only(self):
        summary, params = _parse_docstring("Just a summary.")
        assert summary == "Just a summary."
        assert params == {}

    def test_descriptions_in_schema(self):
        """Docstring param descriptions are included in the JSON schema."""

        @tool
        def documented(query: str, limit: int = 10) -> str:
            """Search for items.

            Args:
                query: The search query.
                limit: Max results to return.
            """
            return ""

        schema = typed_schema(documented)
        assert schema["properties"]["query"]["description"] == "The search query."
        assert schema["properties"]["limit"]["description"] == "Max results to return."


# ---------------------------------------------------------------------------
# Import from apxm top-level
# ---------------------------------------------------------------------------


class TestImports:
    def test_flat_import_tool(self):
        from apxm import tool as t

        assert t is tool

    def test_flat_import_tool_context(self):
        from apxm import ToolContext as TC

        assert TC is ToolContext

    def test_flat_import_function_tool(self):
        from apxm import FunctionTool as FT

        assert FT is FunctionTool


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def typed_schema(ft: FunctionTool) -> dict:
    """Parse the schema_json of a FunctionTool."""
    return json.loads(ft.schema_json)
