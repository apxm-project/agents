"""APXM Agent class — high-level wrapper that lowers to SPAWN_AGENT + REGISTER_CAPABILITY + ASK."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Protocol, runtime_checkable

from . import constants as graph_keys
from .normalize import normalize_value as _normalize_value
from .normalize import normalize_model_id as _normalize_model_id
from .normalize import normalize_provider_spec as _normalize_provider_spec
from .proxy import GraphRecorder, NodeRef

if TYPE_CHECKING:
    from .backends import BackendRoute
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
        route: BackendRoute | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | None = None,
        backend: str | None = None,
        output_schema: type | dict[str, Any] | None = None,
    ) -> None:
        self.name = name
        self.instructions = instructions
        if backend is not None:
            raise TypeError("pass route=select_backend(...) instead of raw backend=")
        if route is not None and model is not None:
            raise ValueError("pass either route= or model=, not both")
        self.route = route
        self.model = model
        self.provider = provider
        self.output_schema = output_schema

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
        _apply_backend_route_attrs(spawn_attrs, self.route, self.model, self.provider)
        if self.instructions is not None:
            spawn_attrs[graph_keys.SYSTEM_PROMPT] = self.instructions
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
            cap_attrs[graph_keys.PYTHON_HANDLER_ID] = tool.handler_id

            cap_node = g._add_node(
                g._auto_name(graph_keys.OP_REGISTER_CAPABILITY),
                graph_keys.OP_REGISTER_CAPABILITY,
                cap_attrs,
            )
            g.add_edge(prev_node, cap_node, dependency=graph_keys.DEPENDENCY_CONTROL)
            prev_node = cap_node

            # Register tool for artifact sidecar embedding
            g.register_python_tool(tool)

        # -- 3. ASK --
        resolved, auto_pairs = g._resolve_template_refs(prompt)

        ask_attrs: dict[str, Any] = {graph_keys.TEMPLATE_STR: resolved}
        if auto_pairs:
            ask_attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        _apply_backend_route_attrs(ask_attrs, self.route, self.model, self.provider)
        if self.instructions is not None:
            ask_attrs[graph_keys.SYSTEM_PROMPT] = self.instructions
        if self.output_schema is not None:
            ask_attrs[graph_keys.OUTPUT_SCHEMA] = _normalize_value(self.output_schema)
        if self._tools:
            ask_attrs[graph_keys.TOOLS] = [tool.name for tool in self._tools]
        ask_attrs.update(
            {k: v for k, v in attributes.items() if v is not None}
        )

        ask_node = g._add_node(
            g._auto_name(graph_keys.OP_ASK),
            graph_keys.OP_ASK,
            ask_attrs,
        )
        g.add_edge(prev_node, ask_node, dependency=graph_keys.DEPENDENCY_CONTROL)

        # Auto-wire data edges from template references
        for _name, ref in auto_pairs:
            g.add_edge(ref, ask_node)

        return ask_node

    def bind(self, g: GraphRecorder) -> BoundAgent:
        """Bind this agent to a recorder, returning a fluent handle.

        The handle emits SPAWN_AGENT + REGISTER_CAPABILITY eagerly, then
        exposes .ask() for multiple prompts against the same agent instance.

        Idempotent per (recorder, agent name): repeated calls return the
        same BoundAgent so the runtime sees a single SPAWN_AGENT per agent.
        """
        existing = g._bound_agents.get(self.name)
        if existing is not None:
            return existing
        bound = BoundAgent(self, g)
        g._bound_agents[self.name] = bound
        return bound

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

        # Emit SPAWN_AGENT eagerly. We stamp instructions+model so the runtime
        # can dispatch HANDOFF/COMMUNICATE against this agent without it being
        # registered as its own compiled flow.
        spawn_attrs: dict[str, Any] = {graph_keys.AGENT_NAME: agent.name}
        _apply_backend_route_attrs(spawn_attrs, agent.route, agent.model, agent.provider)
        if agent.instructions is not None:
            spawn_attrs[graph_keys.SYSTEM_PROMPT] = agent.instructions
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
            cap_attrs[graph_keys.PYTHON_HANDLER_ID] = tool.handler_id

            cap_node = g._add_node(
                g._auto_name(graph_keys.OP_REGISTER_CAPABILITY),
                graph_keys.OP_REGISTER_CAPABILITY,
                cap_attrs,
            )
            g.add_edge(prev_node, cap_node, dependency=graph_keys.DEPENDENCY_CONTROL)
            prev_node = cap_node

            # Register tool for artifact sidecar embedding
            g.register_python_tool(tool)

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
        _apply_backend_route_attrs(ask_attrs, agent.route, agent.model, agent.provider)
        if agent.instructions is not None:
            ask_attrs[graph_keys.SYSTEM_PROMPT] = agent.instructions
        if agent.output_schema is not None:
            ask_attrs[graph_keys.OUTPUT_SCHEMA] = _normalize_value(agent.output_schema)
        if agent._tools:
            ask_attrs[graph_keys.TOOLS] = [tool.name for tool in agent._tools]
        ask_attrs.update(
            {k: v for k, v in attributes.items() if v is not None}
        )

        ask_node = g._add_node(
            g._auto_name(graph_keys.OP_ASK),
            graph_keys.OP_ASK,
            ask_attrs,
        )
        g.add_edge(self._last_node, ask_node, dependency=graph_keys.DEPENDENCY_CONTROL)

        for _name, ref in auto_pairs:
            g.add_edge(ref, ask_node)

        self._last_node = ask_node
        return ask_node

    def handoff(
        self,
        target_agent: Agent,
        payload: str,
        transfer_state: bool = True,
    ) -> NodeRef:
        """Hand off execution from this agent to target_agent.

        Emits a HANDOFF node that transfers control from this bound agent
        to the target. If transfer_state is True, context-stack frames are
        copied from source to target at runtime.

        Returns the HANDOFF NodeRef (the target agent's response token).
        """
        g = self._g

        # Ensure the target agent is also spawned
        target_bound = target_agent.bind(g)

        resolved, auto_pairs = g._resolve_template_refs(payload)

        handoff_attrs: dict[str, Any] = {
            graph_keys.HANDOFF_FROM: self._agent.name,
            graph_keys.HANDOFF_TO: target_agent.name,
            graph_keys.TRANSFER_STATE: transfer_state,
            graph_keys.TEMPLATE_STR: resolved,
        }
        if auto_pairs:
            handoff_attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]

        handoff_node = g._add_node(
            g._auto_name(graph_keys.OP_HANDOFF),
            graph_keys.OP_HANDOFF,
            handoff_attrs,
        )
        # Data edges so the source's last node and the target's SPAWN_AGENT both
        # become MLIR variadic inputs to HANDOFF — Control edges are dropped by
        # to_air() (only Data edges become SSA uses), so they would not enforce
        # ordering on the runtime side and HANDOFF would race the SPAWN_AGENT
        # for the target's STM `_agent_info:<name>` write.
        g.add_edge(self._last_node, handoff_node, dependency=graph_keys.DEPENDENCY_DATA)
        g.add_edge(
            target_bound.get_spawn_node(),
            handoff_node,
            dependency=graph_keys.DEPENDENCY_DATA,
        )

        # Auto-wire data edges from template references
        for _name, ref in auto_pairs:
            g.add_edge(ref, handoff_node)

        self._last_node = handoff_node
        return handoff_node

    def get_spawn_node(self) -> NodeRef:
        """Return the SPAWN_AGENT node."""
        return self._spawn_node

    def __repr__(self) -> str:
        return f"BoundAgent(name={self._agent.name!r})"


def _apply_backend_route_attrs(
    attrs: dict[str, Any],
    route: Any | None,
    model: Any | None,
    provider: Any | None,
) -> None:
    if route is not None:
        from .backends import BackendRoute

        if not isinstance(route, BackendRoute):
            raise TypeError("route must be a BackendRoute returned by select_backend()")
        attrs[graph_keys.BACKEND] = route.backend
        if route.model is not None:
            attrs[graph_keys.MODEL] = route.model
    elif model is not None:
        attrs[graph_keys.MODEL] = _normalize_model_id(model)

    if provider is not None:
        attrs[graph_keys.PROVIDER] = _normalize_provider_spec(provider)


__all__ = ["Agent", "BoundAgent", "ToolLike"]
