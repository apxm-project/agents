from __future__ import annotations

from dataclasses import asdict, is_dataclass
import json
from typing import Any, Iterable

from apxm._generated import constants as c
from . import constants as graph_keys
from .config import AgentConfig
from .ir import ApxmGraph, GraphEdge, GraphNode, Parameter


class NodeRef:
    def __init__(self, recorder: "GraphRecorder", node_id: int, name: str) -> None:
        self._recorder = recorder
        self._node_id = node_id
        self.name = name

    def __rshift__(self, other: "NodeRef") -> "NodeRef":
        self._recorder.add_edge(self, other, dependency="Control")
        return other

    def __or__(self, other: "NodeRef") -> "NodeRef":
        self._recorder.add_edge(self, other, dependency="Data")
        return other

    def __repr__(self) -> str:
        return f"NodeRef(name={self.name!r}, id={self._node_id})"


class GraphRecorder:
    def __init__(self, name: str, *, metadata: dict[str, Any] | None = None) -> None:
        self._name = name
        self._next_id = 1
        self._nodes: list[GraphNode] = []
        self._edges: list[GraphEdge] = []
        self._parameters: list[Parameter] = []
        self._node_ids: dict[str, int] = {}
        self._metadata = (
            dict(metadata)
            if metadata is not None
            else {graph_keys.IS_ENTRY: True}
        )

    def param(self, name: str, type_name: str = "str") -> "GraphRecorder":
        if any(param.name == name for param in self._parameters):
            raise ValueError(f"parameter '{name}' already exists")
        self._parameters.append(Parameter(name=name, type_name=type_name))
        return self

    def add_edge(self, from_ref: NodeRef, to_ref: NodeRef, dependency: str = "Data") -> None:
        self._edges.append(
            GraphEdge(from_id=from_ref._node_id, to_id=to_ref._node_id, dependency=dependency)
        )

    def ask(
        self,
        name: str,
        template: str,
        *,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        attrs = {graph_keys.TEMPLATE_STR: template}
        attrs.update(_compose_system_prompt(agent, "ask"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_ASK, attrs)

    def think(
        self,
        name: str,
        template: str,
        *,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        attrs = {graph_keys.TEMPLATE_STR: template}
        attrs.update(_compose_system_prompt(agent, "think"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_THINK, attrs)

    def reason(
        self,
        name: str,
        template: str,
        *,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        attrs = {graph_keys.TEMPLATE_STR: template}
        attrs.update(_compose_system_prompt(agent, "reason"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_REASON, attrs)

    def query_memory(
        self,
        name: str,
        *,
        query: str,
        space: str | None = None,
        limit: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        attrs: dict[str, Any] = {graph_keys.QUERY: query}
        if space is not None:
            attrs[graph_keys.MEMORY_TIER] = space
        if limit is not None:
            attrs["limit"] = limit
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_QMEM, attrs)

    def update_memory(
        self,
        name: str,
        *,
        data: Any,
        space: str | None = None,
        key: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        attrs: dict[str, Any] = {graph_keys.VALUE: _normalize_value(data), graph_keys.KEY: key or name}
        if space is not None:
            attrs[graph_keys.MEMORY_TIER] = space
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_UMEM, attrs)

    def invoke(
        self,
        name: str,
        *,
        capability: str,
        params: str | dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        attrs: dict[str, Any] = {graph_keys.CAPABILITY: capability}
        if isinstance(params, dict):
            attrs[graph_keys.PARAMS_JSON] = json.dumps(params)
        elif params is not None:
            attrs[graph_keys.PARAMS_JSON] = params
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_INV, attrs)

    def branch(
        self,
        name: str,
        *,
        condition_node: NodeRef | None = None,
        true_label: str,
        false_label: str,
        **attributes: Any,
    ) -> NodeRef:
        node = self._add_node(
            name,
            graph_keys.OP_BRANCH_ON_VALUE,
            _normalize_attributes(
                {
                    graph_keys.TRUE_LABEL: true_label,
                    graph_keys.FALSE_LABEL: false_label,
                    **attributes,
                }
            ),
        )
        if condition_node is not None:
            condition_node | node
        return node

    def switch_(
        self,
        name: str,
        *,
        discriminant: str,
        cases: list[str],
        **attributes: Any,
    ) -> NodeRef:
        attrs = {
            graph_keys.DISCRIMINANT: discriminant,
            graph_keys.CASE_LABELS: list(cases),
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_SWITCH, attrs)

    def wait_all(self, name: str, *dependencies: NodeRef | Iterable[NodeRef]) -> NodeRef:
        flat_dependencies = _flatten_refs(dependencies)
        node = self._add_node(name, graph_keys.OP_WAIT_ALL, {})
        for dep in flat_dependencies:
            dep | node
        return node

    def merge(self, name: str, *dependencies: NodeRef | Iterable[NodeRef]) -> NodeRef:
        flat_dependencies = _flatten_refs(dependencies)
        node = self._add_node(name, graph_keys.OP_MERGE, {})
        for dep in flat_dependencies:
            dep | node
        return node

    def fence(self, name: str, **attributes: Any) -> NodeRef:
        return self._add_node(name, graph_keys.OP_FENCE, _normalize_attributes(attributes))

    def plan(self, name: str, *, goal: str, agent: AgentConfig | None = None, **attributes: Any) -> NodeRef:
        attrs = {graph_keys.GOAL: goal}
        attrs.update(_compose_system_prompt(agent, "plan"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_PLAN, attrs)

    def reflect(self, name: str, *, trace_id: str, agent: AgentConfig | None = None, **attributes: Any) -> NodeRef:
        attrs = {graph_keys.TRACE_ID: trace_id}
        attrs.update(_compose_system_prompt(agent, "reflect"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_REFLECT, attrs)

    def verify(
        self,
        name: str,
        *,
        claim: str,
        evidence: str | None = None,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        condition = claim if evidence is None else f"Claim: {claim}\nEvidence: {evidence}"
        attrs = {graph_keys.CONDITION: condition}
        attrs.update(_compose_system_prompt(agent, "verify"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_VERIFY, attrs)

    def input_guardrail(
        self,
        name: str,
        *,
        schema: dict[str, Any],
        **attributes: Any,
    ) -> NodeRef:
        """Insert a schema-based input guardrail (Verify node)."""
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: json.dumps(schema) if isinstance(schema, dict) else str(schema),
            graph_keys.GUARDRAIL_KIND: "input",
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_VERIFY, attrs)

    def output_guardrail(
        self,
        name: str,
        *,
        schema: dict[str, Any],
        max_retries: int = 3,
        **attributes: Any,
    ) -> NodeRef:
        """Insert a schema-based output guardrail (Verify node with retry semantics)."""
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: json.dumps(schema) if isinstance(schema, dict) else str(schema),
            graph_keys.GUARDRAIL_KIND: "output",
            graph_keys.MAX_RETRIES: max_retries,
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_VERIFY, attrs)

    def handoff(
        self,
        name: str,
        from_agent: NodeRef,
        to_agent: NodeRef,
    ) -> NodeRef:
        """Unconditional handoff: route output from one agent to another.

        Compile-time verified via graph validation — invalid targets are
        caught before execution.
        """
        # Create a data edge from from_agent → to_agent via a pass-through node
        node = self._add_node(name, graph_keys.OP_ASK, {
            graph_keys.TEMPLATE_STR: "{0}",
            graph_keys.HANDOFF: True,
            graph_keys.HANDOFF_FROM: from_agent.name,
            graph_keys.HANDOFF_TO: to_agent.name,
        })
        from_agent | node
        node | to_agent
        return node

    def handoff_when(
        self,
        name: str,
        from_agent: NodeRef,
        routes: dict[str, NodeRef],
    ) -> NodeRef:
        """Conditional handoff: route based on discriminant value.

        All target agents are verified at compile time via graph edges.
        """
        case_labels = list(routes.keys())
        switch_node = self.switch_(
            f"{name}_switch",
            discriminant="{0}",
            cases=case_labels,
        )
        from_agent | switch_node

        for _label, target in routes.items():
            switch_node | target

        return switch_node

    def checkpoint(self, name: str, **attributes: Any) -> NodeRef:
        """Insert a checkpoint barrier (fence with checkpoint semantics).

        When the workflow hits this node during ``run_until_fence()``, execution
        pauses and a serialisable ``WorkflowCheckpoint`` is returned.
        """
        attrs: dict[str, Any] = {graph_keys.CHECKPOINT: True}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_FENCE, attrs)

    def execute(
        self,
        name: str,
        *,
        code: str,
        sandbox_config: dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Execute code in a sandboxed environment (EXC)."""
        attrs: dict[str, Any] = {graph_keys.CODE: code}
        if sandbox_config is not None:
            attrs["sandbox_config"] = _normalize_value(sandbox_config)
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_EXC, attrs)

    def print_(self, name: str, *, message: str, **attributes: Any) -> NodeRef:
        """Print output to stdout (PRINT)."""
        attrs: dict[str, Any] = {graph_keys.MESSAGE: message}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_PRINT, attrs)

    def jump(self, name: str, *, label: str, **attributes: Any) -> NodeRef:
        """Unconditional jump to a labeled instruction (JUMP)."""
        attrs: dict[str, Any] = {graph_keys.LABEL: label}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_JUMP, attrs)

    def loop_start(
        self,
        name: str,
        *,
        count: int,
        **attributes: Any,
    ) -> NodeRef:
        """Begin a bounded loop (LOOP_START)."""
        attrs: dict[str, Any] = {graph_keys.COUNT: count}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_LOOP_START, attrs)

    def loop_end(self, name: str, **attributes: Any) -> NodeRef:
        """End a bounded loop (LOOP_END)."""
        return self._add_node(name, graph_keys.OP_LOOP_END, _normalize_attributes(attributes))

    def return_(
        self,
        name: str,
        *,
        source: NodeRef | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Return from subgraph with a result token (RETURN)."""
        node = self._add_node(name, graph_keys.OP_RETURN, _normalize_attributes(attributes))
        if source is not None:
            source | node
        return node

    def flow_call(
        self,
        name: str,
        *,
        agent_name: str,
        flow_name: str,
        args: dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Call a flow on another agent (FLOW_CALL)."""
        attrs: dict[str, Any] = {
            graph_keys.AGENT_NAME: agent_name,
            graph_keys.FLOW_NAME: flow_name,
        }
        if args is not None:
            attrs["args"] = _normalize_value(args)
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_FLOW_CALL, attrs)

    def try_catch(
        self,
        name: str,
        *,
        try_label: str,
        catch_label: str,
        **attributes: Any,
    ) -> NodeRef:
        """Structured exception handling (TRY_CATCH)."""
        attrs: dict[str, Any] = {
            graph_keys.TRY_LABEL: try_label,
            graph_keys.CATCH_LABEL: catch_label,
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_TRY_CATCH, attrs)

    def err(
        self,
        name: str,
        *,
        error_handler: str,
        **attributes: Any,
    ) -> NodeRef:
        """Error handler invocation (ERR)."""
        attrs: dict[str, Any] = {graph_keys.RECOVERY_TEMPLATE: error_handler}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_ERR, attrs)

    def communicate(
        self,
        name: str,
        *,
        target_agent: str,
        message: str,
        protocol: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Send a message to another agent (COMMUNICATE)."""
        attrs: dict[str, Any] = {
            graph_keys.RECIPIENT: target_agent,
            graph_keys.MESSAGE: message,
        }
        if protocol is not None:
            attrs[graph_keys.PROTOCOL] = protocol
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_COMMUNICATE, attrs)

    def update_goal(
        self,
        name: str,
        *,
        goal_id: str,
        action: str = "set",
        priority: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Modify AAM goals at runtime (UPDATE_GOAL)."""
        attrs: dict[str, Any] = {
            graph_keys.GOAL_ID: goal_id,
            graph_keys.ACTION: action,
        }
        if priority is not None:
            attrs[graph_keys.PRIORITY] = priority
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_UPDATE_GOAL, attrs)

    def guard(
        self,
        name: str,
        *,
        condition: str,
        error_message: str | None = None,
        on_fail: str = "halt",
        source: NodeRef | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Enforce preconditions before execution continues (GUARD)."""
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: condition,
            graph_keys.ON_FAIL: on_fail,
        }
        if error_message is not None:
            attrs[graph_keys.ERROR_MESSAGE] = error_message
        attrs.update(_normalize_attributes(attributes))
        node = self._add_node(name, graph_keys.OP_GUARD, attrs)
        if source is not None:
            source | node
        return node

    def claim(
        self,
        name: str,
        *,
        queue: str,
        lease_ms: int | None = None,
        max_wait_ms: int | None = None,
        server_url: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Atomically claim a task from a shared work queue (CLAIM)."""
        attrs: dict[str, Any] = {graph_keys.QUEUE: queue}
        if lease_ms is not None:
            attrs[graph_keys.LEASE_MS] = lease_ms
        if max_wait_ms is not None:
            attrs[graph_keys.MAX_WAIT_MS] = max_wait_ms
        if server_url is not None:
            attrs[graph_keys.SERVER_URL] = server_url
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_CLAIM, attrs)

    def pause(
        self,
        name: str,
        *,
        message: str,
        checkpoint_id: str | None = None,
        timeout_ms: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Suspend execution pending human-in-the-loop review (PAUSE)."""
        attrs: dict[str, Any] = {graph_keys.MESSAGE: message}
        if checkpoint_id is not None:
            attrs[graph_keys.CHECKPOINT_ID] = checkpoint_id
        if timeout_ms is not None:
            attrs[graph_keys.TIMEOUT_MS] = timeout_ms
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_PAUSE, attrs)

    def resume(
        self,
        name: str,
        *,
        checkpoint: str,
        poll_max_attempts: int | None = None,
        poll_interval_ms: int | None = None,
        server_url: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Resume a suspended PAUSE checkpoint (RESUME)."""
        attrs: dict[str, Any] = {graph_keys.CHECKPOINT: checkpoint}
        if poll_max_attempts is not None:
            attrs[graph_keys.POLL_MAX_ATTEMPTS] = poll_max_attempts
        if poll_interval_ms is not None:
            attrs[graph_keys.POLL_INTERVAL_MS] = poll_interval_ms
        if server_url is not None:
            attrs[graph_keys.SERVER_URL] = server_url
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_RESUME, attrs)

    def agent(
        self,
        name: str,
        *,
        memory: dict[str, Any] | None = None,
        beliefs: dict[str, Any] | None = None,
        goals: list[str] | None = None,
        capabilities: list[str] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Agent metadata declaration (AGENT)."""
        attrs: dict[str, Any] = {}
        if memory is not None:
            attrs["memory"] = _normalize_value(memory)
        if beliefs is not None:
            attrs["beliefs"] = _normalize_value(beliefs)
        if goals is not None:
            attrs["goals"] = _normalize_value(goals)
        if capabilities is not None:
            attrs["capabilities"] = _normalize_value(capabilities)
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_AGENT, attrs)

    def const_(self, name: str, value: Any, **attributes: Any) -> NodeRef:
        """String constant, compiler internal (CONST_STR)."""
        attrs = {graph_keys.VALUE: _normalize_value(value)}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_CONST_STR, attrs)

    def yield_(self, name: str, *, source: NodeRef | None = None, **attributes: Any) -> NodeRef:
        """Yield value from a switch-case region, compiler internal (YIELD)."""
        node = self._add_node(name, graph_keys.OP_YIELD, _normalize_attributes(attributes))
        if source is not None:
            source | node
        return node

    def delegate(
        self,
        name: str,
        *,
        task_spec: str,
        target_agent: str,
        **attributes: Any,
    ) -> NodeRef:
        """Delegate a task to a sub-agent for execution (DELEGATE)."""
        attrs: dict[str, Any] = {
            graph_keys.TASK_SPEC: task_spec,
            graph_keys.TARGET_AGENT: target_agent,
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_DELEGATE, attrs)

    def negotiate(
        self,
        name: str,
        *,
        parties: list[str],
        proposal: str,
        max_rounds: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Multi-agent negotiation protocol for consensus building (NEGOTIATE)."""
        attrs: dict[str, Any] = {
            graph_keys.PARTIES: list(parties),
            graph_keys.PROPOSAL: proposal,
        }
        if max_rounds is not None:
            attrs[graph_keys.MAX_ROUNDS] = max_rounds
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_NEGOTIATE, attrs)

    def nop(self, name: str, **attributes: Any) -> NodeRef:
        """No-op passthrough with no side effects or AAM transition (NOP)."""
        return self._add_node(name, graph_keys.OP_NOP, _normalize_attributes(attributes))

    def identity(self, name: str, **attributes: Any) -> NodeRef:
        """Identity passthrough that records an AAM identity transition (IDENTITY)."""
        return self._add_node(name, graph_keys.OP_IDENTITY, _normalize_attributes(attributes))

    def spawn_agent(
        self,
        name: str,
        *,
        agent_name: str,
        profile: str | None = None,
        mode: str | None = None,
        model: str | None = None,
        cwd: str | None = None,
        capabilities: list[str] | None = None,
        goals: list[str] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Create a new agent instance at runtime (SPAWN_AGENT)."""
        attrs: dict[str, Any] = {graph_keys.AGENT_NAME: agent_name}
        if profile is not None:
            attrs[graph_keys.PROFILE] = profile
        if mode is not None:
            attrs[graph_keys.MODE] = mode
        if model is not None:
            attrs[graph_keys.MODEL] = model
        if cwd is not None:
            attrs[graph_keys.CWD] = cwd
        if capabilities is not None:
            attrs["capabilities"] = _normalize_value(capabilities)
        if goals is not None:
            attrs["goals"] = _normalize_value(goals)
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_SPAWN_AGENT, attrs)

    def register_capability(
        self,
        name: str,
        *,
        capability_name: str,
        description: str | None = None,
        parameters_schema: str | dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Register a new capability in the runtime registry (REGISTER_CAPABILITY)."""
        attrs: dict[str, Any] = {graph_keys.CAPABILITY_NAME: capability_name}
        if description is not None:
            attrs[graph_keys.DESCRIPTION] = description
        if parameters_schema is not None:
            if isinstance(parameters_schema, dict):
                attrs[graph_keys.PARAMETERS_SCHEMA] = json.dumps(parameters_schema)
            else:
                attrs[graph_keys.PARAMETERS_SCHEMA] = parameters_schema
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_REGISTER_CAPABILITY, attrs)

    def autonomous(
        self,
        name: str,
        *,
        region: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Switch a sub-graph region to model-driven execution (AUTONOMOUS, stub)."""
        attrs: dict[str, Any] = {}
        if region is not None:
            attrs[graph_keys.REGION] = region
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_AUTONOMOUS, attrs)

    def to_graph(self) -> ApxmGraph:
        return ApxmGraph(
            name=self._name,
            nodes=list(self._nodes),
            edges=list(self._edges),
            parameters=list(self._parameters),
            metadata=dict(self._metadata),
        )

    def to_air(self) -> str:
        """Emit canonical .air text IR for this graph.

        Equivalent to: self.to_graph().to_air()
        """
        return self.to_graph().to_air()

    def _add_node(self, name: str, op: str, attributes: dict[str, Any]) -> NodeRef:
        if name in self._node_ids:
            raise ValueError(f"workflow node '{name}' already exists")

        node_id = self._next_id
        self._next_id += 1

        node = GraphNode(id=node_id, name=name, op=op, attributes=attributes)
        self._nodes.append(node)
        self._node_ids[name] = node_id
        return NodeRef(self, node_id, name)


def _flatten_refs(items: tuple[NodeRef | Iterable[NodeRef], ...]) -> list[NodeRef]:
    refs: list[NodeRef] = []
    for item in items:
        if isinstance(item, NodeRef):
            refs.append(item)
            continue

        for nested in item:
            if not isinstance(nested, NodeRef):
                raise TypeError(f"expected NodeRef, got {type(nested)!r}")
            refs.append(nested)
    return refs


def _compose_system_prompt(agent: AgentConfig | None, op: str) -> dict[str, Any]:
    """Compose system_prompt from agent base + operation-specific instruction."""
    if agent is None:
        return {}

    attrs = _normalize_attributes(agent.to_node_attributes())

    if agent.operation_instructions:
        op_key = op.lower()
        op_instruction = agent.operation_instructions.get(op_key)
        if op_instruction:
            base = agent.system_prompt or ""
            if base:
                composed = f"{base}\n\n{op_instruction}"
            else:
                composed = op_instruction
            attrs[graph_keys.SYSTEM_PROMPT] = composed

    return attrs


def _normalize_attributes(attributes: dict[str, Any]) -> dict[str, Any]:
    return {key: _normalize_value(value) for key, value in attributes.items() if value is not None}


def _normalize_value(value: Any) -> Any:
    if hasattr(value, "to_dict") and callable(value.to_dict):
        return value.to_dict()
    if is_dataclass(value):
        return asdict(value)
    if isinstance(value, tuple):
        return [_normalize_value(v) for v in value]
    if isinstance(value, list):
        return [_normalize_value(v) for v in value]
    if isinstance(value, dict):
        return {k: _normalize_value(v) for k, v in value.items()}
    return value
