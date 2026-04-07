from __future__ import annotations

from dataclasses import asdict, is_dataclass
import inspect
import json
import re
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
        self._name_counters: dict[str, int] = {}
        self._metadata = (
            dict(metadata)
            if metadata is not None
            else {graph_keys.IS_ENTRY: True}
        )
        # Track parameter names for auto-wiring resolution
        self._param_names: set[str] = set()

    def _auto_name(self, op_type: str) -> str:
        """Generate a unique name based on operation type and counter."""
        count = self._name_counters.get(op_type, 0)
        self._name_counters[op_type] = count + 1
        return op_type.lower() if count == 0 else f"{op_type.lower()}_{count}"

    def param(self, name: str, type_name: str = "str") -> "GraphRecorder":
        if any(param.name == name for param in self._parameters):
            raise ValueError(f"parameter '{name}' already exists")
        self._parameters.append(Parameter(name=name, type_name=type_name))
        self._param_names.add(name)
        return self

    def add_edge(self, from_ref: NodeRef, to_ref: NodeRef, dependency: str = "Data") -> None:
        self._edges.append(
            GraphEdge(from_id=from_ref._node_id, to_id=to_ref._node_id, dependency=dependency)
        )

    def _resolve_template_refs(self, template: str) -> tuple[str, list[NodeRef]]:
        """Replace {var_name} with {N} and collect auto-wire edges.

        Looks up variable names in the caller's local scope. If a variable
        holds a NodeRef or AgentHandle, auto-creates a data edge and replaces
        the template placeholder with a positional index.

        Skips:
        - Compile parameters (e.g., {task} in a @compile flow)
        - Already-positional refs (e.g., {0}, {1})

        Returns:
            Tuple of (resolved_template, list_of_NodeRefs_to_wire)
        """
        # Get caller's locals (2 frames back: this method -> calling method -> user code)
        caller_frame = inspect.currentframe()
        if caller_frame is None:
            return template, []

        caller_locals: dict[str, Any] = {}
        try:
            # Go up 2 frames: _resolve_template_refs -> ask/think/etc -> user code
            if caller_frame.f_back and caller_frame.f_back.f_back:
                caller_locals = caller_frame.f_back.f_back.f_locals
        finally:
            del caller_frame

        refs: list[NodeRef] = []

        def replacer(match: re.Match[str]) -> str:
            var_name = match.group(1)

            # Skip compile parameter names
            if var_name in self._param_names:
                return match.group(0)

            # Skip already-positional {0}, {1}, etc.
            if var_name.isdigit():
                return match.group(0)

            # Look up in caller locals
            val = caller_locals.get(var_name)

            # Handle NodeRef
            if isinstance(val, NodeRef):
                idx = len(refs)
                refs.append(val)
                return f"{{{idx}}}"

            # Handle AgentHandle (has get_last_node method)
            if hasattr(val, 'get_last_node') and callable(val.get_last_node):
                idx = len(refs)
                refs.append(val.get_last_node())
                return f"{{{idx}}}"

            # Not found or not a node reference - leave as-is
            return match.group(0)

        resolved = re.sub(r'\{(\w+)\}', replacer, template)
        return resolved, refs

    def ask(
        self,
        name_or_template: str | None = None,
        /,
        *,
        name: str | None = None,
        template: str | None = None,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        # Heuristic: if first positional arg contains {, it's a template
        if name_or_template is not None:
            if '{' in name_or_template:
                template = name_or_template
            else:
                name = name_or_template

        if name is None:
            name = self._auto_name("ask")
        if template is None:
            raise ValueError("ask() missing required keyword argument: 'template'")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_template, auto_refs = self._resolve_template_refs(template)

        attrs = {graph_keys.TEMPLATE_STR: resolved_template}
        attrs.update(_compose_system_prompt(agent, "ask"))
        attrs.update(_normalize_attributes(attributes))
        node = self._add_node(name, graph_keys.OP_ASK, attrs)

        # Create auto-wire edges
        for ref in auto_refs:
            ref | node

        return node

    def think(
        self,
        name_or_template: str | None = None,
        /,
        *,
        name: str | None = None,
        template: str | None = None,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        # Heuristic: if first positional arg contains {, it's a template
        if name_or_template is not None:
            if '{' in name_or_template:
                template = name_or_template
            else:
                name = name_or_template

        if name is None:
            name = self._auto_name("think")
        if template is None:
            raise ValueError("think() missing required keyword argument: 'template'")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_template, auto_refs = self._resolve_template_refs(template)

        attrs = {graph_keys.TEMPLATE_STR: resolved_template}
        attrs.update(_compose_system_prompt(agent, "think"))
        attrs.update(_normalize_attributes(attributes))
        node = self._add_node(name, graph_keys.OP_THINK, attrs)

        # Create auto-wire edges
        for ref in auto_refs:
            ref | node

        return node

    def reason(
        self,
        name_or_template: str | None = None,
        /,
        *,
        name: str | None = None,
        template: str | None = None,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        # Heuristic: if first positional arg contains {, it's a template
        if name_or_template is not None:
            if '{' in name_or_template:
                template = name_or_template
            else:
                name = name_or_template

        if name is None:
            name = self._auto_name("reason")
        if template is None:
            raise ValueError("reason() missing required keyword argument: 'template'")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_template, auto_refs = self._resolve_template_refs(template)

        attrs = {graph_keys.TEMPLATE_STR: resolved_template}
        attrs.update(_compose_system_prompt(agent, "reason"))
        attrs.update(_normalize_attributes(attributes))
        node = self._add_node(name, graph_keys.OP_REASON, attrs)

        # Create auto-wire edges
        for ref in auto_refs:
            ref | node

        return node

    def query_memory(
        self,
        name: str | None = None,
        *,
        query: str | None = None,
        space: str | None = None,
        limit: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name("query_memory")
        if query is None:
            raise ValueError("query_memory() missing required keyword argument: 'query'")
        attrs: dict[str, Any] = {graph_keys.QUERY: query}
        if space is not None:
            attrs[graph_keys.MEMORY_TIER] = space
        if limit is not None:
            attrs["limit"] = limit
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_QMEM, attrs)

    def update_memory(
        self,
        name: str | None = None,
        *,
        data: Any = None,
        space: str | None = None,
        key: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name("update_memory")
        if data is None:
            raise ValueError("update_memory() missing required keyword argument: 'data'")
        attrs: dict[str, Any] = {graph_keys.VALUE: _normalize_value(data), graph_keys.KEY: key or name}
        if space is not None:
            attrs[graph_keys.MEMORY_TIER] = space
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_UMEM, attrs)

    def invoke(
        self,
        name: str | None = None,
        *,
        capability: str | None = None,
        params: str | dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name("invoke")
        if capability is None:
            raise ValueError("invoke() missing required keyword argument: 'capability'")
        attrs: dict[str, Any] = {graph_keys.CAPABILITY: capability}
        if isinstance(params, dict):
            attrs[graph_keys.PARAMS_JSON] = json.dumps(params)
        elif params is not None:
            attrs[graph_keys.PARAMS_JSON] = params
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_INV, attrs)

    def branch(
        self,
        name: str | None = None,
        *,
        condition_node: NodeRef | None = None,
        true_label: str | None = None,
        false_label: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name("branch")
        if true_label is None:
            raise ValueError("branch() missing required keyword argument: 'true_label'")
        if false_label is None:
            raise ValueError("branch() missing required keyword argument: 'false_label'")
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
        name: str | None = None,
        *,
        discriminant: str | None = None,
        cases: list[str] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name("switch")
        if discriminant is None:
            raise ValueError("switch_() missing required keyword argument: 'discriminant'")
        if cases is None:
            raise ValueError("switch_() missing required keyword argument: 'cases'")
        attrs = {
            graph_keys.DISCRIMINANT: discriminant,
            graph_keys.CASE_LABELS: list(cases),
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_SWITCH, attrs)

    def wait_all(self, name: str | None = None, *dependencies: NodeRef | Iterable[NodeRef]) -> NodeRef:
        if name is None:
            name = self._auto_name("wait_all")
        flat_dependencies = _flatten_refs(dependencies)
        node = self._add_node(name, graph_keys.OP_WAIT_ALL, {})
        for dep in flat_dependencies:
            dep | node
        return node

    def merge(self, name: str | None = None, *dependencies: NodeRef | Iterable[NodeRef]) -> NodeRef:
        if name is None:
            name = self._auto_name("merge")
        flat_dependencies = _flatten_refs(dependencies)
        node = self._add_node(name, graph_keys.OP_MERGE, {})
        for dep in flat_dependencies:
            dep | node
        return node

    def fence(self, name: str | None = None, **attributes: Any) -> NodeRef:
        if name is None:
            name = self._auto_name("fence")
        return self._add_node(name, graph_keys.OP_FENCE, _normalize_attributes(attributes))

    def plan(self, name: str | None = None, *, goal: str | None = None, agent: AgentConfig | None = None, **attributes: Any) -> NodeRef:
        if name is None:
            name = self._auto_name("plan")
        if goal is None:
            raise ValueError("plan() missing required keyword argument: 'goal'")
        attrs = {graph_keys.GOAL: goal}
        attrs.update(_compose_system_prompt(agent, "plan"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_PLAN, attrs)

    def reflect(self, name: str | None = None, *, trace_id: str | None = None, agent: AgentConfig | None = None, **attributes: Any) -> NodeRef:
        if name is None:
            name = self._auto_name("reflect")
        if trace_id is None:
            raise ValueError("reflect() missing required keyword argument: 'trace_id'")
        attrs = {graph_keys.TRACE_ID: trace_id}
        attrs.update(_compose_system_prompt(agent, "reflect"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_REFLECT, attrs)

    def verify(
        self,
        name: str | None = None,
        *,
        claim: str | None = None,
        evidence: str | None = None,
        agent: AgentConfig | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name("verify")
        if claim is None:
            raise ValueError("verify() missing required keyword argument: 'claim'")
        condition = claim if evidence is None else f"Claim: {claim}\nEvidence: {evidence}"
        attrs = {graph_keys.CONDITION: condition}
        attrs.update(_compose_system_prompt(agent, "verify"))
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_VERIFY, attrs)

    def input_guardrail(
        self,
        name: str | None = None,
        *,
        schema: dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Insert a schema-based input guardrail (Verify node)."""
        if name is None:
            name = self._auto_name("input_guardrail")
        if schema is None:
            raise ValueError("input_guardrail() missing required keyword argument: 'schema'")
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: json.dumps(schema) if isinstance(schema, dict) else str(schema),
            graph_keys.GUARDRAIL_KIND: "input",
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_VERIFY, attrs)

    def output_guardrail(
        self,
        name: str | None = None,
        *,
        schema: dict[str, Any] | None = None,
        max_retries: int = 3,
        **attributes: Any,
    ) -> NodeRef:
        """Insert a schema-based output guardrail (Verify node with retry semantics)."""
        if name is None:
            name = self._auto_name("output_guardrail")
        if schema is None:
            raise ValueError("output_guardrail() missing required keyword argument: 'schema'")
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: json.dumps(schema) if isinstance(schema, dict) else str(schema),
            graph_keys.GUARDRAIL_KIND: "output",
            graph_keys.MAX_RETRIES: max_retries,
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_VERIFY, attrs)

    def handoff(
        self,
        name: str | None = None,
        *,
        from_agent: NodeRef | None = None,
        to_agent: NodeRef | None = None,
    ) -> NodeRef:
        """Unconditional handoff: route output from one agent to another.

        Compile-time verified via graph validation — invalid targets are
        caught before execution.
        """
        if name is None:
            name = self._auto_name("handoff")
        if from_agent is None:
            raise ValueError("handoff() missing required keyword argument: 'from_agent'")
        if to_agent is None:
            raise ValueError("handoff() missing required keyword argument: 'to_agent'")
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
        name: str | None = None,
        *,
        from_agent: NodeRef | None = None,
        routes: dict[str, NodeRef] | None = None,
    ) -> NodeRef:
        """Conditional handoff: route based on discriminant value.

        All target agents are verified at compile time via graph edges.
        """
        if name is None:
            name = self._auto_name("handoff_when")
        if from_agent is None:
            raise ValueError("handoff_when() missing required keyword argument: 'from_agent'")
        if routes is None:
            raise ValueError("handoff_when() missing required keyword argument: 'routes'")
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

    def checkpoint(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """Insert a checkpoint barrier (fence with checkpoint semantics).

        When the workflow hits this node during ``run_until_fence()``, execution
        pauses and a serialisable ``WorkflowCheckpoint`` is returned.
        """
        if name is None:
            name = self._auto_name("checkpoint")
        attrs: dict[str, Any] = {graph_keys.CHECKPOINT: True}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_FENCE, attrs)

    def execute(
        self,
        name: str | None = None,
        *,
        code: str | None = None,
        sandbox_config: dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Execute code in a sandboxed environment (EXC)."""
        if name is None:
            name = self._auto_name("execute")
        if code is None:
            raise ValueError("execute() missing required keyword argument: 'code'")
        attrs: dict[str, Any] = {graph_keys.CODE: code}
        if sandbox_config is not None:
            attrs["sandbox_config"] = _normalize_value(sandbox_config)
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_EXC, attrs)

    def print_(
        self,
        name_or_message: str | None = None,
        /,
        *,
        name: str | None = None,
        message: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Print output to stdout (PRINT)."""
        # Heuristic: if first positional arg contains {, it's a message
        if name_or_message is not None:
            if '{' in name_or_message:
                message = name_or_message
            else:
                name = name_or_message

        if name is None:
            name = self._auto_name("print")
        if message is None:
            raise ValueError("print_() missing required keyword argument: 'message'")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_message, auto_refs = self._resolve_template_refs(message)

        attrs: dict[str, Any] = {graph_keys.MESSAGE: resolved_message}
        attrs.update(_normalize_attributes(attributes))
        node = self._add_node(name, graph_keys.OP_PRINT, attrs)

        # Create auto-wire edges
        for ref in auto_refs:
            ref | node

        return node

    # Alias - 'print' is fine as a method name (just not as a function name)
    print = print_

    def jump(self, name: str | None = None, *, label: str | None = None, **attributes: Any) -> NodeRef:
        """Unconditional jump to a labeled instruction (JUMP)."""
        if name is None:
            name = self._auto_name("jump")
        if label is None:
            raise ValueError("jump() missing required keyword argument: 'label'")
        attrs: dict[str, Any] = {graph_keys.LABEL: label}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_JUMP, attrs)

    def loop_start(
        self,
        name: str | None = None,
        *,
        count: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Begin a bounded loop (LOOP_START)."""
        if name is None:
            name = self._auto_name("loop_start")
        if count is None:
            raise ValueError("loop_start() missing required keyword argument: 'count'")
        attrs: dict[str, Any] = {graph_keys.COUNT: count}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_LOOP_START, attrs)

    def loop_end(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """End a bounded loop (LOOP_END)."""
        if name is None:
            name = self._auto_name("loop_end")
        return self._add_node(name, graph_keys.OP_LOOP_END, _normalize_attributes(attributes))

    def return_(
        self,
        name: str | None = None,
        *,
        source: NodeRef | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Return from subgraph with a result token (RETURN)."""
        if name is None:
            name = self._auto_name("return")
        node = self._add_node(name, graph_keys.OP_RETURN, _normalize_attributes(attributes))
        if source is not None:
            source | node
        return node

    def done(
        self,
        source: NodeRef | None = None,
        name: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Alias for return_() - cleaner name for workflow completion."""
        return self.return_(name=name, source=source, **attributes)

    def flow_call(
        self,
        name: str | None = None,
        *,
        agent_name: str | None = None,
        flow_name: str | None = None,
        args: dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Call a flow on another agent (FLOW_CALL)."""
        if name is None:
            name = self._auto_name("flow_call")
        if agent_name is None:
            raise ValueError("flow_call() missing required keyword argument: 'agent_name'")
        if flow_name is None:
            raise ValueError("flow_call() missing required keyword argument: 'flow_name'")
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
        name: str | None = None,
        *,
        try_label: str | None = None,
        catch_label: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Structured exception handling (TRY_CATCH)."""
        if name is None:
            name = self._auto_name("try_catch")
        if try_label is None:
            raise ValueError("try_catch() missing required keyword argument: 'try_label'")
        if catch_label is None:
            raise ValueError("try_catch() missing required keyword argument: 'catch_label'")
        attrs: dict[str, Any] = {
            graph_keys.TRY_LABEL: try_label,
            graph_keys.CATCH_LABEL: catch_label,
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_TRY_CATCH, attrs)

    def err(
        self,
        name: str | None = None,
        *,
        error_handler: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Error handler invocation (ERR)."""
        if name is None:
            name = self._auto_name("err")
        if error_handler is None:
            raise ValueError("err() missing required keyword argument: 'error_handler'")
        attrs: dict[str, Any] = {graph_keys.RECOVERY_TEMPLATE: error_handler}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_ERR, attrs)

    def communicate(
        self,
        name_or_message: str | None = None,
        /,
        *,
        name: str | None = None,
        target_agent: str | None = None,
        message: str | None = None,
        protocol: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Send a message to another agent (COMMUNICATE)."""
        # Heuristic: if first positional arg contains {, it's a message
        if name_or_message is not None:
            if '{' in name_or_message:
                message = name_or_message
            else:
                name = name_or_message

        if name is None:
            name = self._auto_name("communicate")
        if target_agent is None:
            raise ValueError("communicate() missing required keyword argument: 'target_agent'")
        if message is None:
            raise ValueError("communicate() missing required keyword argument: 'message'")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_message, auto_refs = self._resolve_template_refs(message)

        attrs: dict[str, Any] = {
            graph_keys.RECIPIENT: target_agent,
            graph_keys.MESSAGE: resolved_message,
        }
        if protocol is not None:
            attrs[graph_keys.PROTOCOL] = protocol
        attrs.update(_normalize_attributes(attributes))
        node = self._add_node(name, graph_keys.OP_COMMUNICATE, attrs)

        # Create auto-wire edges
        for ref in auto_refs:
            ref | node

        return node

    def update_goal(
        self,
        name: str | None = None,
        *,
        goal_id: str | None = None,
        action: str = "set",
        priority: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Modify AAM goals at runtime (UPDATE_GOAL)."""
        if name is None:
            name = self._auto_name("update_goal")
        if goal_id is None:
            raise ValueError("update_goal() missing required keyword argument: 'goal_id'")
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
        name: str | None = None,
        *,
        condition: str | None = None,
        error_message: str | None = None,
        on_fail: str = "halt",
        source: NodeRef | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Enforce preconditions before execution continues (GUARD)."""
        if name is None:
            name = self._auto_name("guard")
        if condition is None:
            raise ValueError("guard() missing required keyword argument: 'condition'")
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
        name: str | None = None,
        *,
        queue: str | None = None,
        lease_ms: int | None = None,
        max_wait_ms: int | None = None,
        server_url: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Atomically claim a task from a shared work queue (CLAIM)."""
        if name is None:
            name = self._auto_name("claim")
        if queue is None:
            raise ValueError("claim() missing required keyword argument: 'queue'")
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
        name: str | None = None,
        *,
        message: str | None = None,
        checkpoint_id: str | None = None,
        timeout_ms: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Suspend execution pending human-in-the-loop review (PAUSE)."""
        if name is None:
            name = self._auto_name("pause")
        if message is None:
            raise ValueError("pause() missing required keyword argument: 'message'")
        attrs: dict[str, Any] = {graph_keys.MESSAGE: message}
        if checkpoint_id is not None:
            attrs[graph_keys.CHECKPOINT_ID] = checkpoint_id
        if timeout_ms is not None:
            attrs[graph_keys.TIMEOUT_MS] = timeout_ms
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_PAUSE, attrs)

    def resume(
        self,
        name: str | None = None,
        *,
        checkpoint: str | None = None,
        poll_max_attempts: int | None = None,
        poll_interval_ms: int | None = None,
        server_url: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Resume a suspended PAUSE checkpoint (RESUME)."""
        if name is None:
            name = self._auto_name("resume")
        if checkpoint is None:
            raise ValueError("resume() missing required keyword argument: 'checkpoint'")
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
        name: str | None = None,
        *,
        memory: dict[str, Any] | None = None,
        beliefs: dict[str, Any] | None = None,
        goals: list[str] | None = None,
        capabilities: list[str] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Agent metadata declaration (AGENT)."""
        if name is None:
            name = self._auto_name("agent")
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

    def const_(self, name: str | None = None, *, value: Any = None, **attributes: Any) -> NodeRef:
        """String constant, compiler internal (CONST_STR). DEPRECATED: Use text() or string() instead."""
        if name is None:
            name = self._auto_name("const_str")
        if value is None:
            raise ValueError("const_() missing required keyword argument: 'value'")
        attrs = {graph_keys.VALUE: _normalize_value(value)}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_CONST_STR, attrs)

    def text(self, name: str | None = None, *, value: Any = None, **attributes: Any) -> NodeRef:
        """String constant (CONST_STR)."""
        if name is None:
            name = self._auto_name("text")
        if value is None:
            raise ValueError("text() missing required keyword argument: 'value'")
        attrs = {graph_keys.VALUE: _normalize_value(value)}
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_CONST_STR, attrs)

    def string(self, name: str | None = None, *, value: Any = None, **attributes: Any) -> NodeRef:
        """String constant (CONST_STR). Alias for text()."""
        return self.text(name, value=value, **attributes)

    def yield_(self, name: str | None = None, *, source: NodeRef | None = None, **attributes: Any) -> NodeRef:
        """Yield value from a switch-case region, compiler internal (YIELD)."""
        if name is None:
            name = self._auto_name("yield")
        node = self._add_node(name, graph_keys.OP_YIELD, _normalize_attributes(attributes))
        if source is not None:
            source | node
        return node

    def delegate(
        self,
        name: str | None = None,
        *,
        task_spec: str | None = None,
        target_agent: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Delegate a task to a sub-agent for execution (DELEGATE)."""
        if name is None:
            name = self._auto_name("delegate")
        if task_spec is None:
            raise ValueError("delegate() missing required keyword argument: 'task_spec'")
        if target_agent is None:
            raise ValueError("delegate() missing required keyword argument: 'target_agent'")
        attrs: dict[str, Any] = {
            graph_keys.TASK_SPEC: task_spec,
            graph_keys.TARGET_AGENT: target_agent,
        }
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_DELEGATE, attrs)

    def negotiate(
        self,
        name: str | None = None,
        *,
        parties: list[str] | None = None,
        proposal: str | None = None,
        max_rounds: int | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Multi-agent negotiation protocol for consensus building (NEGOTIATE)."""
        if name is None:
            name = self._auto_name("negotiate")
        if parties is None:
            raise ValueError("negotiate() missing required keyword argument: 'parties'")
        if proposal is None:
            raise ValueError("negotiate() missing required keyword argument: 'proposal'")
        attrs: dict[str, Any] = {
            graph_keys.PARTIES: list(parties),
            graph_keys.PROPOSAL: proposal,
        }
        if max_rounds is not None:
            attrs[graph_keys.MAX_ROUNDS] = max_rounds
        attrs.update(_normalize_attributes(attributes))
        return self._add_node(name, graph_keys.OP_NEGOTIATE, attrs)

    def nop(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """No-op passthrough with no side effects or AAM transition (NOP)."""
        if name is None:
            name = self._auto_name("nop")
        return self._add_node(name, graph_keys.OP_NOP, _normalize_attributes(attributes))

    def identity(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """Identity passthrough that records an AAM identity transition (IDENTITY)."""
        if name is None:
            name = self._auto_name("identity")
        return self._add_node(name, graph_keys.OP_IDENTITY, _normalize_attributes(attributes))

    def spawn_agent(
        self,
        name: str | None = None,
        *,
        agent_name: str | None = None,
        profile: str | Any | None = None,
        mode: str | None = None,
        model: str | None = None,
        cwd: str | None = None,
        capabilities: list[str] | None = None,
        goals: list[str] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Create a new agent instance at runtime (SPAWN_AGENT).

        Args:
            name: Node name (auto-generated if not provided)
            agent_name: Name of the agent instance
            profile: Agent profile (string name or AgentRef object from apxm._generated.agents)
            mode: Agent mode (e.g., "ask", "explore")
            model: Model name
            cwd: Working directory for the agent
            capabilities: List of capabilities
            goals: List of goals
        """
        if name is None:
            name = self._auto_name("spawn_agent")
        if agent_name is None:
            raise ValueError("spawn_agent() missing required keyword argument: 'agent_name'")
        attrs: dict[str, Any] = {graph_keys.AGENT_NAME: agent_name}
        if profile is not None:
            # Support both string and typed AgentRef from apxm._generated.agents
            if isinstance(profile, str):
                attrs[graph_keys.PROFILE] = profile
            elif hasattr(profile, 'name'):
                # AgentRef or similar typed object
                attrs[graph_keys.PROFILE] = profile.name
            else:
                attrs[graph_keys.PROFILE] = str(profile)
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

    # Shorthand alias
    spawn = spawn_agent

    def register_capability(
        self,
        name: str | None = None,
        *,
        capability_name: str | None = None,
        description: str | None = None,
        parameters_schema: str | dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Register a new capability in the runtime registry (REGISTER_CAPABILITY)."""
        if name is None:
            name = self._auto_name("register_capability")
        if capability_name is None:
            raise ValueError("register_capability() missing required keyword argument: 'capability_name'")
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
        name: str | None = None,
        *,
        region: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Switch a sub-graph region to model-driven execution (AUTONOMOUS, stub)."""
        if name is None:
            name = self._auto_name("autonomous")
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
