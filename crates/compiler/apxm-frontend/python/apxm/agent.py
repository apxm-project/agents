"""APXM Agent class — high-level wrapper that lowers to SPAWN_AGENT + REGISTER_CAPABILITY + ASK."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any, Protocol, runtime_checkable

from . import constants as graph_keys
from .normalize import normalize_value as _normalize_value
from .normalize import normalize_provider as _normalize_provider
from .proxy import GraphRecorder, NodeRef

if TYPE_CHECKING:
    from ._generated.models import ModelId
    from ._generated.providers import ProviderSpec


@runtime_checkable
class ToolLike(Protocol):
    """Minimal interface that FunctionTool satisfies."""

    @property
    def name(self) -> str: ...

    @property
    def description(self) -> str: ...

    @property
    def schema_json(self) -> str: ...

    @property
    def handler_id(self) -> str: ...


@dataclass(slots=True)
class AgentHooks:
    """Lifecycle hooks for agent execution (placeholder for MVP)."""

    on_start: Any | None = None
    on_tool_call: Any | None = None
    on_end: Any | None = None


class Agent:
    """Declarative agent definition that lowers to AIR ops at graph-capture time.

    Usage::

        from apxm import tool, Agent, compile, run, ModelId

        @tool
        def add(a: int, b: int) -> int:
            \"\"\"Add two integers.\"\"\"
            return a + b

        calc = Agent(name="calc", instructions="Do math.", tools=[add],
                     model=ModelId.ANTHROPIC_CLAUDE_OPUS_4)

        @compile()
        def math_flow(g, q: str):
            g.done(calc.ask(g, q))

        print(run(math_flow, "What is 17 + 25?"))
    """

    def __init__(
        self,
        name: str,
        *,
        instructions: str | None = None,
        tools: list[Any] | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | str | None = None,
        backend: str | None = None,
        output_schema: type | dict[str, Any] | None = None,
        hooks: AgentHooks | None = None,
    ) -> None:
        self.name = name
        self.instructions = instructions
        self.model = model
        self.provider = provider
        self.backend = backend
        self.output_schema = output_schema
        self.hooks = hooks

        # Validate and store tools
        self._tools: list[ToolLike] = []
        if tools:
            for t in tools:
                if not isinstance(t, ToolLike):
                    raise TypeError(
                        f"Agent tool must be a @tool-decorated function (FunctionTool), "
                        f"got {type(t).__name__}"
                    )
                self._tools.append(t)

    def ask(
        self,
        g: GraphRecorder,
        prompt: str,
        **attributes: Any,
    ) -> NodeRef:
        """Emit AIR ops for a single agent invocation.

        Lowers to:
          1. SPAWN_AGENT  (agent_name, model, instructions)
          2. REGISTER_CAPABILITY x N  (one per tool, with python_handler_id)
          3. ASK  (prompt, model, system_prompt)

        Returns the ASK NodeRef (the agent's response token).
        """
        # -- 1. SPAWN_AGENT --
        spawn_attrs: dict[str, Any] = {graph_keys.AGENT_NAME: self.name}
        if self.model is not None:
            spawn_attrs[graph_keys.MODEL] = _normalize_value(self.model)
        spawn_node = g._add_node(
            g._auto_name(graph_keys.OP_SPAWN_AGENT),
            graph_keys.OP_SPAWN_AGENT,
            spawn_attrs,
        )

        prev_node = spawn_node

        # -- 2. REGISTER_CAPABILITY x N --
        for tool in self._tools:
            cap_attrs: dict[str, Any] = {
                graph_keys.CAPABILITY_NAME: tool.name,
            }
            if tool.description:
                cap_attrs[graph_keys.DESCRIPTION] = tool.description
            if tool.schema_json:
                cap_attrs[graph_keys.PARAMETERS_SCHEMA] = tool.schema_json
            # python_handler_id links capability to the Python function at runtime
            cap_attrs["python_handler_id"] = tool.handler_id

            cap_node = g._add_node(
                g._auto_name(graph_keys.OP_REGISTER_CAPABILITY),
                graph_keys.OP_REGISTER_CAPABILITY,
                cap_attrs,
            )
            g.add_edge(prev_node, cap_node, dependency="Control")
            prev_node = cap_node

        # -- 3. ASK --
        resolved, auto_pairs = g._resolve_template_refs(prompt)

        ask_attrs: dict[str, Any] = {graph_keys.TEMPLATE_STR: resolved}
        if auto_pairs:
            ask_attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        if self.model is not None:
            ask_attrs[graph_keys.MODEL] = _normalize_value(self.model)
        if self.instructions is not None:
            ask_attrs[graph_keys.SYSTEM_PROMPT] = self.instructions
        if self.provider is not None:
            ask_attrs[graph_keys.PROVIDER] = _normalize_provider(self.provider)
        if self.backend is not None:
            ask_attrs[graph_keys.BACKEND] = self.backend
        if self.output_schema is not None:
            ask_attrs[graph_keys.OUTPUT_SCHEMA] = _normalize_value(self.output_schema)
        ask_attrs.update(
            {k: v for k, v in attributes.items() if v is not None}
        )

        ask_node = g._add_node(
            g._auto_name(graph_keys.OP_ASK),
            graph_keys.OP_ASK,
            ask_attrs,
        )
        g.add_edge(prev_node, ask_node, dependency="Control")

        # Auto-wire data edges from template references
        for _name, ref in auto_pairs:
            g.add_edge(ref, ask_node)

        return ask_node

    def bind(self, g: GraphRecorder) -> BoundAgent:
        """Bind this agent to a recorder, returning a fluent handle.

        The handle emits SPAWN_AGENT + REGISTER_CAPABILITY eagerly, then
        exposes .ask() for multiple prompts against the same agent instance.
        """
        return BoundAgent(self, g)

    def __repr__(self) -> str:
        tool_names = [t.name for t in self._tools]
        return f"Agent(name={self.name!r}, tools={tool_names!r})"


class BoundAgent:
    """Fluent handle for an Agent bound to a GraphRecorder.

    Created by Agent.bind(g). Emits SPAWN_AGENT + REGISTER_CAPABILITY once,
    then allows multiple .ask() calls against the same agent instance.
    """

    def __init__(self, agent: Agent, g: GraphRecorder) -> None:
        self._agent = agent
        self._g = g
        self._msg_counter = 0

        # Emit SPAWN_AGENT eagerly
        spawn_attrs: dict[str, Any] = {graph_keys.AGENT_NAME: agent.name}
        if agent.model is not None:
            spawn_attrs[graph_keys.MODEL] = _normalize_value(agent.model)
        self._spawn_node = g._add_node(
            g._auto_name(graph_keys.OP_SPAWN_AGENT),
            graph_keys.OP_SPAWN_AGENT,
            spawn_attrs,
        )

        prev_node = self._spawn_node

        # Emit REGISTER_CAPABILITY for each tool
        for tool in agent._tools:
            cap_attrs: dict[str, Any] = {
                graph_keys.CAPABILITY_NAME: tool.name,
            }
            if tool.description:
                cap_attrs[graph_keys.DESCRIPTION] = tool.description
            if tool.schema_json:
                cap_attrs[graph_keys.PARAMETERS_SCHEMA] = tool.schema_json
            cap_attrs["python_handler_id"] = tool.handler_id

            cap_node = g._add_node(
                g._auto_name(graph_keys.OP_REGISTER_CAPABILITY),
                graph_keys.OP_REGISTER_CAPABILITY,
                cap_attrs,
            )
            g.add_edge(prev_node, cap_node, dependency="Control")
            prev_node = cap_node

        self._last_node = prev_node

    def ask(self, prompt: str, **attributes: Any) -> NodeRef:
        """Emit an ASK node against this bound agent instance.

        Returns the ASK NodeRef.
        """
        self._msg_counter += 1
        g = self._g
        agent = self._agent

        resolved, auto_pairs = g._resolve_template_refs(prompt)

        ask_attrs: dict[str, Any] = {graph_keys.TEMPLATE_STR: resolved}
        if auto_pairs:
            ask_attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        if agent.model is not None:
            ask_attrs[graph_keys.MODEL] = _normalize_value(agent.model)
        if agent.instructions is not None:
            ask_attrs[graph_keys.SYSTEM_PROMPT] = agent.instructions
        if agent.provider is not None:
            ask_attrs[graph_keys.PROVIDER] = _normalize_provider(agent.provider)
        if agent.backend is not None:
            ask_attrs[graph_keys.BACKEND] = agent.backend
        if agent.output_schema is not None:
            ask_attrs[graph_keys.OUTPUT_SCHEMA] = _normalize_value(agent.output_schema)
        ask_attrs.update(
            {k: v for k, v in attributes.items() if v is not None}
        )

        ask_node = g._add_node(
            g._auto_name(graph_keys.OP_ASK),
            graph_keys.OP_ASK,
            ask_attrs,
        )
        g.add_edge(self._last_node, ask_node, dependency="Control")

        for _name, ref in auto_pairs:
            g.add_edge(ref, ask_node)

        self._last_node = ask_node
        return ask_node

    def get_spawn_node(self) -> NodeRef:
        """Return the SPAWN_AGENT node."""
        return self._spawn_node

    def get_last_node(self) -> NodeRef:
        """Return the most recent node in this agent's chain."""
        return self._last_node

    def __repr__(self) -> str:
        return f"BoundAgent(name={self._agent.name!r})"


__all__ = ["Agent", "AgentHooks", "BoundAgent", "ToolLike"]
