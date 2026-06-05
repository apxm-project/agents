from __future__ import annotations

import inspect
import json
import os
import re
from typing import TYPE_CHECKING, Any, Iterable, Mapping

if TYPE_CHECKING:
    from ._generated.agents import AgentRef
    from ._generated.models import ModelId
    from ._generated.providers import ProviderSpec
    from .backends import BackendRoute

from apxm._generated import constants as c
from . import constants as graph_keys
from .config import AgentConfig, NodePolicy, WorkflowTargetKind
from .normalize import normalize_model_id as _normalize_model_id
from .normalize import normalize_attributes as _normalize_attributes
from .normalize import normalize_provider_spec as _normalize_provider_spec
from .normalize import normalize_value as _normalize_value
from .ir import ApxmGraph, GraphEdge, GraphNode, Parameter
from .tools import FunctionTool

_TEMPLATE_PLACEHOLDER_RE = re.compile(r"\{(\w+)\}")
_MAX_TEMPLATE_SCOPE_DEPTH = 32


class NodeRef:
    def __init__(self, recorder: "GraphRecorder", node_id: int, name: str) -> None:
        self._recorder = recorder
        self._node_id = node_id
        self.name = name

    def __repr__(self) -> str:
        return f"NodeRef(name={self.name!r}, id={self._node_id})"


class GraphRecorder:
    def __init__(
        self,
        name: str,
        *,
        metadata: dict[str, Any] | None = None,
        policy: NodePolicy | dict[str, Any] | None = None,
    ) -> None:
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
        # Accumulate Python tool descriptors for artifact sidecar
        self._python_tools: list[dict[str, Any]] = []
        self._python_tool_ids: set[str] = set()  # dedup by handler_id
        self._python_tool_registration_nodes: dict[str, NodeRef] = {}
        # Track bound agents by name so Agent.bind()/handoff() are idempotent —
        # without this, calling bind() twice (or handoff() against an Agent
        # that was already bound) emits a duplicate SPAWN_AGENT, which the
        # runtime rejects with "Agent already exists in process table".
        self._bound_agents: dict[str, Any] = {}
        self._agent_session_nodes: dict[str, NodeRef] = {}
        self._default_policy = _coerce_node_policy(policy)

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

    def add_edge(
        self,
        from_ref: NodeRef,
        to_ref: NodeRef,
        dependency: str = graph_keys.DEPENDENCY_DATA,
    ) -> None:
        self._edges.append(
            GraphEdge(from_id=from_ref._node_id, to_id=to_ref._node_id, dependency=dependency)
        )

    def _resolve_template_refs(
        self,
        template: str,
        *,
        template_scope: Mapping[str, Any] | None = None,
    ) -> tuple[str, list[tuple[str, NodeRef]]]:
        """Collect auto-wire edges from `{name}` placeholders in template.

        Walks the caller's local scope to look up each `{name}`. For each
        name that resolves to a NodeRef, records `(name, ref)`
        in edge order. The template itself is returned unchanged — every
        `{name}` placeholder remains a named reference (the runtime
        substitutes by looking up `name` in the node's `input_names`).

        Names not bound to a NodeRef/AgentHandle (compile params, literal
        unknowns) are not added to the returned list. The compiler validator
        is responsible for catching unresolved placeholders.

        Returns:
            (template_unchanged, [(name, NodeRef), ...] in edge order)
        """
        scope_chain: Iterable[Mapping[str, Any]]
        if template_scope is None:
            scope_chain = self._template_scope_chain()
        else:
            scope_chain = (template_scope,)

        pairs: list[tuple[str, NodeRef]] = []
        seen: set[str] = set()

        for match in _TEMPLATE_PLACEHOLDER_RE.finditer(template):
            var_name = match.group(1)
            if var_name in seen:
                continue
            seen.add(var_name)

            # Compile parameters are resolved at execute time against
            # module.parameters, not via input_names — skip wiring.
            if var_name in self._param_names:
                continue

            val = self._resolve_name_from_scope_chain(var_name, scope_chain)
            if isinstance(val, NodeRef):
                pairs.append((var_name, val))
            # Otherwise leave as-is — validator will diagnose if unresolved.

        return template, pairs

    @staticmethod
    def _template_scope_chain() -> tuple[Mapping[str, Any], ...]:
        caller_frame = inspect.currentframe()
        if caller_frame is None:
            return ()

        scopes: list[Mapping[str, Any]] = []
        try:
            frame = caller_frame.f_back
            if frame is not None:
                frame = frame.f_back
            if frame is not None:
                frame = frame.f_back

            depth = 0
            while frame is not None and depth < _MAX_TEMPLATE_SCOPE_DEPTH:
                scopes.append(frame.f_locals)
                frame = frame.f_back
                depth += 1
        finally:
            del caller_frame

        return tuple(scopes)

    @staticmethod
    def _resolve_name_from_scope_chain(
        name: str,
        scope_chain: Iterable[Mapping[str, Any]],
    ) -> Any:
        for scope in scope_chain:
            if name in scope:
                return scope[name]
        return None

    def _bind_flow_kwargs(
        self,
        parameters: list[Parameter],
        kwargs: dict[str, Any],
    ) -> list[tuple[str, Any]]:
        """Bind caller kwargs onto the callee parameter names strictly by name."""
        if not parameters:
            return list(kwargs.items())

        param_names = [param.name for param in parameters]
        unknown = [key for key in kwargs if key not in param_names]
        if unknown:
            expected = ", ".join(param_names)
            raise TypeError(
                f"call() received unknown argument(s) for flow '{self._name}': "
                f"{', '.join(unknown)}. Expected [{expected}]"
            )

        missing = [name for name in param_names if name not in kwargs]
        if missing:
            raise TypeError(f"missing required argument(s): {', '.join(missing)}")

        return [(param_name, kwargs[param_name]) for param_name in param_names]

    def _split_invocation_args(
        self,
        items: Iterable[tuple[str, Any]],
    ) -> tuple[dict[str, Any], list[tuple[str, NodeRef]]]:
        literal_args: dict[str, Any] = {}
        node_ref_args: list[tuple[str, NodeRef]] = []

        for key, val in items:
            if isinstance(val, NodeRef):
                node_ref_args.append((key, val))
                literal_args[key] = f"{{{key}}}"
            else:
                literal_args[key] = val

        return literal_args, node_ref_args

    def _apply_policy(
        self,
        base_attrs: dict[str, Any],
        attributes: dict[str, Any],
    ) -> dict[str, Any]:
        explicit_attrs = dict(attributes)
        inline_policy = _extract_inline_node_policy(explicit_attrs)
        resolved_policy = (
            self._default_policy.overlay(inline_policy)
            if self._default_policy is not None
            else inline_policy
        )

        merged: dict[str, Any] = {}
        if resolved_policy is not None:
            merged.update(resolved_policy.to_node_attributes())
        merged.update(base_attrs)
        merged.update(_normalize_attributes(explicit_attrs))
        return merged

    def ask(
        self,
        *,
        name: str | None = None,
        prompt: str | None = None,
        agent: AgentConfig | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | None = None,
        route: BackendRoute | None = None,
        backend: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_ASK)
        if prompt is None:
            raise ValueError("ask() missing required argument: prompt")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved, auto_pairs = self._resolve_template_refs(prompt)

        attrs: dict[str, Any] = {graph_keys.TEMPLATE_STR: resolved}
        if auto_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        _apply_routing_attrs(attrs, route=route, model=model, provider=provider, backend=backend)
        attrs.update(_compose_system_prompt(agent, graph_keys.OP_ASK))
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_ASK, attrs)

        # Create auto-wire edges (in input_names order)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)

        return node

    def think(
        self,
        *,
        name: str | None = None,
        prompt: str | None = None,
        agent: AgentConfig | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | None = None,
        route: BackendRoute | None = None,
        backend: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_THINK)
        if prompt is None:
            raise ValueError("think() missing required argument: prompt")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved, auto_pairs = self._resolve_template_refs(prompt)

        attrs: dict[str, Any] = {graph_keys.TEMPLATE_STR: resolved}
        if auto_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        _apply_routing_attrs(attrs, route=route, model=model, provider=provider, backend=backend)
        attrs.update(_compose_system_prompt(agent, graph_keys.OP_THINK))
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_THINK, attrs)

        # Create auto-wire edges (in input_names order)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)

        return node

    def reason(
        self,
        *,
        name: str | None = None,
        prompt: str | None = None,
        agent: AgentConfig | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | None = None,
        route: BackendRoute | None = None,
        backend: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_REASON)
        if prompt is None:
            raise ValueError("reason() missing required argument: prompt")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved, auto_pairs = self._resolve_template_refs(prompt)

        attrs: dict[str, Any] = {graph_keys.TEMPLATE_STR: resolved}
        if auto_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        _apply_routing_attrs(attrs, route=route, model=model, provider=provider, backend=backend)
        attrs.update(_compose_system_prompt(agent, graph_keys.OP_REASON))
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_REASON, attrs)

        # Create auto-wire edges (in input_names order)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)

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
            name = self._auto_name(graph_keys.OP_QMEM)
        if query is None:
            raise ValueError("query_memory() missing required keyword argument: 'query'")
        attrs: dict[str, Any] = {graph_keys.QUERY: query}
        if space is not None:
            attrs[graph_keys.MEMORY_TIER] = space
        if limit is not None:
            attrs[graph_keys.LIMIT] = limit
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_UMEM)
        if data is None:
            raise ValueError("update_memory() missing required keyword argument: 'data'")
        attrs: dict[str, Any] = {graph_keys.VALUE: _normalize_value(data), graph_keys.KEY: key or name}
        if space is not None:
            attrs[graph_keys.MEMORY_TIER] = space
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_INV_TOOL)
        if capability is None:
            raise ValueError("invoke() missing required keyword argument: 'capability'")
        attrs: dict[str, Any] = {graph_keys.CAPABILITY: capability}
        params_str: str | None = None
        if isinstance(params, dict):
            params_str = json.dumps(params)
        elif params is not None:
            params_str = params
        # Auto-wire `{name}` placeholders in the params JSON exactly like ask/think
        # do for the prompt: each `{name}` bound to a NodeRef in scope becomes a
        # Data operand + an `input_names` entry, so the runtime substitutes it.
        # (JSON structural braces never match the placeholder regex.)
        auto_pairs: list[tuple[str, NodeRef]] = []
        if params_str is not None:
            resolved, auto_pairs = self._resolve_template_refs(params_str)
            attrs[graph_keys.PARAMS_JSON] = resolved
            if auto_pairs:
                attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_INV_TOOL, attrs)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)
        return node

    def register_tool(self, tool: FunctionTool, name: str | None = None, **attributes: Any) -> NodeRef:
        """Register a Python @tool function as a runtime capability."""
        if not isinstance(tool, FunctionTool):
            raise TypeError("register_tool() expects a @tool-decorated FunctionTool")

        existing = self._python_tool_registration_nodes.get(tool.handler_id)
        if existing is not None:
            return existing

        node_name = name or self._auto_name(graph_keys.OP_REGISTER_CAPABILITY)
        register = self.register_capability(
            node_name,
            capability_name=tool.name,
            description=tool.description,
            parameters_schema=tool.schema_json,
            python_handler_id=tool.handler_id,
            **attributes,
        )
        self.register_python_tool(tool)
        self._python_tool_registration_nodes[tool.handler_id] = register
        return register

    def invoke_tool(
        self,
        tool: FunctionTool,
        name: str | None = None,
        *,
        params: str | dict[str, Any] | None = None,
        node_attributes: dict[str, Any] | None = None,
        **tool_args: Any,
    ) -> NodeRef:
        """Register and invoke a Python @tool function.

        Keyword arguments are treated as tool parameters so simple tools can be
        called as ``g.invoke_tool(search_docs, query="...")``. Use ``params``
        when a tool parameter name conflicts with recorder options.
        """
        if not isinstance(tool, FunctionTool):
            raise TypeError("invoke_tool() expects a @tool-decorated FunctionTool")
        if params is not None and tool_args:
            raise TypeError("invoke_tool() accepts either params= or keyword tool arguments, not both")

        invocation_params = params if params is not None else tool_args or None
        invocation_attributes = dict(node_attributes or {})

        registration = self.register_tool(tool)
        invocation = self.invoke(
            name,
            capability=tool.name,
            params=invocation_params,
            **invocation_attributes,
        )
        self.add_edge(
            registration,
            invocation,
            dependency=graph_keys.DEPENDENCY_CONTROL,
        )
        return invocation

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
            name = self._auto_name(graph_keys.OP_BRANCH_ON_VALUE)
        if true_label is None:
            raise ValueError("branch() missing required keyword argument: 'true_label'")
        if false_label is None:
            raise ValueError("branch() missing required keyword argument: 'false_label'")
        node = self._add_node(
            name,
            graph_keys.OP_BRANCH_ON_VALUE,
            self._apply_policy(
                {
                    graph_keys.TRUE_LABEL: true_label,
                    graph_keys.FALSE_LABEL: false_label,
                },
                attributes,
            ),
        )
        if condition_node is not None:
            self.add_edge(condition_node, node)
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
            name = self._auto_name(graph_keys.OP_SWITCH)
        if discriminant is None:
            raise ValueError("switch_() missing required keyword argument: 'discriminant'")
        if cases is None:
            raise ValueError("switch_() missing required keyword argument: 'cases'")
        attrs = {
            graph_keys.DISCRIMINANT: discriminant,
            graph_keys.CASE_LABELS: list(cases),
        }
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_SWITCH, attrs)

    def wait_all(self, name: str | None = None, *dependencies: NodeRef | Iterable[NodeRef]) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_WAIT_ALL)
        flat_dependencies = _flatten_refs(dependencies)
        node = self._add_node(name, graph_keys.OP_WAIT_ALL, {})
        for dep in flat_dependencies:
            self.add_edge(dep, node)
        return node

    def merge(self, name: str | None = None, *dependencies: NodeRef | Iterable[NodeRef]) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_MERGE)
        flat_dependencies = _flatten_refs(dependencies)
        node = self._add_node(name, graph_keys.OP_MERGE, {})
        for dep in flat_dependencies:
            self.add_edge(dep, node)
        return node

    def fence(self, name: str | None = None, **attributes: Any) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_FENCE)
        return self._add_node(name, graph_keys.OP_FENCE, self._apply_policy({}, attributes))

    def plan(self, name: str | None = None, *, goal: str | None = None, agent: AgentConfig | None = None, model: ModelId | None = None, provider: ProviderSpec | None = None, route: BackendRoute | None = None, backend: str | None = None, **attributes: Any) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_PLAN)
        if goal is None:
            raise ValueError("plan() missing required keyword argument: 'goal'")
        resolved_goal, auto_pairs = self._resolve_template_refs(goal)
        attrs: dict[str, Any] = {graph_keys.GOAL: resolved_goal}
        if auto_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        _apply_routing_attrs(attrs, route=route, model=model, provider=provider, backend=backend)
        attrs.update(_compose_system_prompt(agent, graph_keys.OP_PLAN))
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_PLAN, attrs)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)
        return node

    def reflect(self, name: str | None = None, *, trace_id: str | None = None, agent: AgentConfig | None = None, model: ModelId | None = None, provider: ProviderSpec | None = None, route: BackendRoute | None = None, backend: str | None = None, **attributes: Any) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_REFLECT)
        if trace_id is None:
            raise ValueError("reflect() missing required keyword argument: 'trace_id'")
        resolved_trace_id, auto_pairs = self._resolve_template_refs(trace_id)
        attrs: dict[str, Any] = {graph_keys.TRACE_ID: resolved_trace_id}
        if auto_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        _apply_routing_attrs(attrs, route=route, model=model, provider=provider, backend=backend)
        attrs.update(_compose_system_prompt(agent, graph_keys.OP_REFLECT))
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_REFLECT, attrs)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)
        return node

    def verify(
        self,
        name: str | None = None,
        *,
        claim: str | None = None,
        evidence: str | None = None,
        agent: AgentConfig | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | None = None,
        route: BackendRoute | None = None,
        backend: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if name is None:
            name = self._auto_name(graph_keys.OP_VERIFY)
        if claim is None:
            raise ValueError("verify() missing required keyword argument: 'claim'")
        resolved_claim, claim_pairs = self._resolve_template_refs(claim)
        all_pairs: list[tuple[str, NodeRef]] = list(claim_pairs)
        if evidence is not None:
            resolved_evidence, ev_pairs = self._resolve_template_refs(evidence)
            # Dedupe by name (an input shouldn't appear twice in input_names)
            seen_names = {n for n, _ in all_pairs}
            for n, r in ev_pairs:
                if n not in seen_names:
                    all_pairs.append((n, r))
                    seen_names.add(n)
            attrs: dict[str, Any] = {
                graph_keys.CLAIM_TEXT: resolved_claim,
                graph_keys.EVIDENCE: resolved_evidence,
            }
        else:
            attrs = {graph_keys.CLAIM_TEXT: resolved_claim}
        if all_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in all_pairs]
        _apply_routing_attrs(attrs, route=route, model=model, provider=provider, backend=backend)
        attrs.update(_compose_system_prompt(agent, graph_keys.OP_VERIFY))
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_VERIFY, attrs)
        for _name, ref in all_pairs:
            self.add_edge(ref, node)
        return node

    def input_guardrail(
        self,
        name: str | None = None,
        *,
        schema: dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Insert a schema-based input guardrail (Verify node)."""
        if name is None:
            name = self._auto_name("input_" + graph_keys.OP_VERIFY)
        if schema is None:
            raise ValueError("input_guardrail() missing required keyword argument: 'schema'")
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: json.dumps(schema) if isinstance(schema, dict) else str(schema),
            graph_keys.GUARDRAIL_KIND: "input",
        }
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name("output_" + graph_keys.OP_VERIFY)
        if schema is None:
            raise ValueError("output_guardrail() missing required keyword argument: 'schema'")
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: json.dumps(schema) if isinstance(schema, dict) else str(schema),
            graph_keys.GUARDRAIL_KIND: "output",
            graph_keys.MAX_RETRIES: max_retries,
        }
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_VERIFY, attrs)

    def checkpoint(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """Insert a checkpoint barrier (fence with checkpoint semantics).

        When the workflow hits this node during ``run_until_fence()``, execution
        pauses and a serialisable ``WorkflowCheckpoint`` is returned.
        """
        if name is None:
            name = self._auto_name(graph_keys.OP_CHECKPOINT)
        attrs: dict[str, Any] = {graph_keys.CHECKPOINT: True}
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_EXC)
        if code is None:
            raise ValueError("execute() missing required keyword argument: 'code'")
        attrs: dict[str, Any] = {graph_keys.CODE: code}
        if sandbox_config is not None:
            attrs[graph_keys.SANDBOX_CONFIG] = _normalize_value(sandbox_config)
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_EXC, attrs)

    def print(
        self,
        *,
        name: str | None = None,
        message: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Print output to stdout (PRINT)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_PRINT)
        if message is None:
            raise ValueError("print() missing required keyword argument: 'message'")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_message, auto_pairs = self._resolve_template_refs(message)

        attrs: dict[str, Any] = {graph_keys.MESSAGE: resolved_message}
        if auto_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_PRINT, attrs)

        # Create auto-wire edges (in input_names order)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)

        return node

    def jump(self, name: str | None = None, *, label: str | None = None, **attributes: Any) -> NodeRef:
        """Unconditional jump to a labeled instruction (JUMP)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_JUMP)
        if label is None:
            raise ValueError("jump() missing required keyword argument: 'label'")
        attrs: dict[str, Any] = {graph_keys.LABEL: label}
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_JUMP, attrs)

    def loop_start(
        self,
        name: str | None = None,
        *,
        count: int | None = None,
        label: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Begin a bounded loop (LOOP_START).

        The iteration bound is written as ``max_iterations`` because that is
        the attribute the runtime handler (loop_start.rs) actually reads;
        ``count_token`` is also stamped so the AIS OpSpec/validator agree.
        """
        if name is None:
            name = self._auto_name(graph_keys.OP_LOOP_START)
        if count is None:
            raise ValueError("loop_start() missing required keyword argument: 'count'")
        attrs: dict[str, Any] = {
            graph_keys.MAX_ITERATIONS: count,
            graph_keys.COUNT_TOKEN: str(count),
        }
        if label is not None:
            attrs[graph_keys.LABEL] = label
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_LOOP_START, attrs)

    def loop_end(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """End a bounded loop (LOOP_END)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_LOOP_END)
        return self._add_node(name, graph_keys.OP_LOOP_END, self._apply_policy({}, attributes))

    def done(
        self,
        source: NodeRef | None = None,
        name: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Return from subgraph with a result token (RETURN)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_RETURN)
        node = self._add_node(name, graph_keys.OP_RETURN, self._apply_policy({}, attributes))
        if source is not None:
            self.add_edge(source, node)
        return node

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
            name = self._auto_name(graph_keys.OP_FLOW_CALL)
        if agent_name is None:
            raise ValueError("flow_call() missing required keyword argument: 'agent_name'")
        if flow_name is None:
            raise ValueError("flow_call() missing required keyword argument: 'flow_name'")
        attrs: dict[str, Any] = {
            graph_keys.AGENT_NAME: agent_name,
            graph_keys.FLOW_NAME: flow_name,
        }
        if args is not None:
            literal_args, node_ref_args = self._split_invocation_args(args.items())
            attrs[graph_keys.ARGS] = _normalize_value(literal_args)
            if node_ref_args:
                attrs[graph_keys.INPUT_NAMES] = [param_name for param_name, _ in node_ref_args]
        else:
            node_ref_args = []
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_FLOW_CALL, attrs)
        for _param_name, ref in node_ref_args:
            self.add_edge(ref, node)
        return node

    def workflow_spawn(
        self,
        name: str | None = None,
        *,
        target_kind: WorkflowTargetKind | None = None,
        target: str | os.PathLike[str] | None = None,
        args: dict[str, Any] | None = None,
        session_root: str | os.PathLike[str] | None = None,
        await_result: bool = True,
        node_policy: NodePolicy | dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Spawn a child graph, artifact, or workflow execution."""
        if name is None:
            name = self._auto_name(graph_keys.OP_WORKFLOW_SPAWN)
        if target_kind is None:
            raise ValueError("workflow_spawn() missing required keyword argument: 'target_kind'")
        if not isinstance(target_kind, WorkflowTargetKind):
            raise TypeError("workflow_spawn() target_kind must be a WorkflowTargetKind")
        if target_kind not in WorkflowTargetKind.spawn_path_kinds():
            expected = "', '".join(kind.value for kind in WorkflowTargetKind.spawn_path_kinds())
            raise ValueError(
                "workflow_spawn() target_kind must be one of: "
                f"'{expected}'"
            )
        if target is None:
            raise ValueError("workflow_spawn() missing required keyword argument: 'target'")
        if not await_result:
            raise ValueError("workflow_spawn() requires await_result=True")

        attrs: dict[str, Any] = {
            graph_keys.TARGET_KIND: target_kind.value,
            graph_keys.TARGET: os.fspath(target),
            graph_keys.AWAIT_RESULT: True,
        }
        if session_root is not None:
            attrs[graph_keys.SESSION_ROOT] = os.fspath(session_root)
        if args is not None:
            literal_args, node_ref_args = self._split_invocation_args(args.items())
            attrs[graph_keys.ARGS] = _normalize_value(literal_args)
            if node_ref_args:
                attrs[graph_keys.INPUT_NAMES] = [param_name for param_name, _ in node_ref_args]
        else:
            node_ref_args = []

        if node_policy is not None:
            attrs = self._apply_policy(attrs, {"node_policy": node_policy, **attributes})
        else:
            attrs = self._apply_policy(attrs, attributes)

        node = self._add_node(name, graph_keys.OP_WORKFLOW_SPAWN, attrs)
        for _param_name, ref in node_ref_args:
            self.add_edge(ref, node)
        return node

    def call(
        self,
        flow: Any,
        *,
        name: str | None = None,
        node_policy: NodePolicy | dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> NodeRef:
        """Invoke a compiled flow or FlowModule from this graph.

        Accepts a @compile-decorated function, a FlowModule instance, or
        an ApxmGraph. Keyword arguments are mapped to the flow's parameters.

        Args:
            flow: A @compile function, FlowModule, or ApxmGraph to invoke
            name: Node name (auto-generated if not provided)
            **kwargs: Arguments passed to the called flow's parameters.
                      Values can be NodeRef (auto-wired as data edges) or
                      literal values (serialized into the args dict).
        """
        # Extract graph from various sources
        if hasattr(flow, '_graph'):
            # @compile-decorated _CompiledFunction
            graph = flow._graph
        elif hasattr(flow, 'nodes') and hasattr(flow, 'edges') and hasattr(flow, 'name'):
            # ApxmGraph directly
            graph = flow
        elif hasattr(flow, 'to_graph') and callable(flow.to_graph):
            # FlowModule or similar
            graph = flow.to_graph()
        else:
            raise TypeError(
                f"call() expects a @compile function, FlowModule, or ApxmGraph, "
                f"got {type(flow).__name__}"
            )

        flow_name = graph.name
        if name is None:
            name = self._auto_name(f"call_{flow_name}")

        bound_kwargs = self._bind_flow_kwargs(graph.parameters, kwargs)
        literal_args, node_ref_args = self._split_invocation_args(bound_kwargs)

        attrs: dict[str, Any] = {
            graph_keys.AGENT_NAME: flow_name,
            graph_keys.FLOW_NAME: "main",
        }
        if literal_args:
            attrs[graph_keys.ARGS] = _normalize_value(literal_args)
        # Stamp input_names parallel to the Data edges below; the args dict
        # references each input by `{param_name}`.
        if node_ref_args:
            attrs[graph_keys.INPUT_NAMES] = [pn for pn, _ in node_ref_args]

        if node_policy is not None:
            attrs = self._apply_policy(attrs, {"node_policy": node_policy})
        node = self._add_node(name, graph_keys.OP_FLOW_CALL, attrs)

        # Auto-wire NodeRef arguments as data edges (in input_names order)
        for _param_name, ref in node_ref_args:
            self.add_edge(ref, node)

        return node

    def call_skill(
        self,
        skill_id: str,
        *,
        name: str | None = None,
        args: dict[str, Any] | None = None,
        version: str | None = None,
        node_policy: NodePolicy | dict[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Invoke an installed skill by id (CALL_SKILL).

        Resolves ``skill_id`` through the runtime SkillLibrary, hash-verifies
        the child artifact, and dispatches it as a child execution. Args follow
        the same convention as :meth:`call`: NodeRef values auto-wire as Data
        edges (referenced as ``{param}`` in the serialized args dict) and
        literals are serialized inline.

        Args:
            skill_id: Installed skill identifier (optionally ``id@version``).
            args: Arguments for the skill's parameters (NodeRef or literal).
            version: Optional version, folded into ``skill_id`` as ``id@version``.
        """
        if not skill_id:
            raise ValueError("call_skill() missing required argument: skill_id")
        if name is None:
            name = self._auto_name(graph_keys.OP_CALL_SKILL)
        resolved_id = (
            f"{skill_id}@{version}"
            if version is not None and "@" not in skill_id
            else skill_id
        )
        attrs: dict[str, Any] = {graph_keys.SKILL_ID: resolved_id}
        if args is not None:
            literal_args, node_ref_args = self._split_invocation_args(args.items())
            attrs[graph_keys.ARGS] = _normalize_value(literal_args)
            if node_ref_args:
                attrs[graph_keys.INPUT_NAMES] = [pn for pn, _ in node_ref_args]
        else:
            node_ref_args = []
        if node_policy is not None:
            attrs = self._apply_policy(attrs, {"node_policy": node_policy, **attributes})
        else:
            attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_CALL_SKILL, attrs)
        for _pn, ref in node_ref_args:
            self.add_edge(ref, node)
        return node

    def embed(
        self,
        flow: Any,
        *,
        prefix: str | None = None,
    ) -> NodeRef:
        """Inline-compose another graph into this one at compile time.

        Copies all nodes, edges, and parameters from the source graph into
        this graph (with optional name prefix). Returns the terminal NodeRef.

        Args:
            flow: A @compile function, FlowModule, or ApxmGraph
            prefix: Optional prefix for namespacing embedded nodes
        """
        from .module import FlowModule as _FlowModule

        if isinstance(flow, _FlowModule):
            return flow.embed(self, prefix=prefix)

        # Extract graph from @compile function or ApxmGraph
        if hasattr(flow, '_graph'):
            source_graph = flow._graph
        elif hasattr(flow, 'nodes') and hasattr(flow, 'edges'):
            source_graph = flow
        elif hasattr(flow, 'to_graph') and callable(flow.to_graph):
            source_graph = flow.to_graph()
        else:
            raise TypeError(
                f"embed() expects a @compile function, FlowModule, or ApxmGraph, "
                f"got {type(flow).__name__}"
            )

        from .ir import GraphEdge as _GE, Parameter as _P
        node_id_map: dict[int, NodeRef] = {}

        for node in source_graph.nodes:
            merged_name = node.name if prefix is None else f"{prefix}_{node.name}"
            merged_ref = self._add_node(merged_name, node.op, dict(node.attributes))
            node_id_map[node.id] = merged_ref

        for edge in source_graph.edges:
            self._edges.append(
                _GE(
                    from_id=node_id_map[edge.from_id]._node_id,
                    to_id=node_id_map[edge.to_id]._node_id,
                    dependency=edge.dependency,
                )
            )

        for param in source_graph.parameters:
            merged_param = param.name if prefix is None else f"{prefix}_{param.name}"
            if any(existing.name == merged_param for existing in self._parameters):
                continue
            self._parameters.append(_P(name=merged_param, type_name=param.type_name))

        # Return the last node as the terminal
        if source_graph.nodes:
            last_node_id = source_graph.nodes[-1].id
            return node_id_map[last_node_id]
        raise ValueError("cannot embed an empty graph")

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
            name = self._auto_name(graph_keys.OP_TRY_CATCH)
        if try_label is None:
            raise ValueError("try_catch() missing required keyword argument: 'try_label'")
        if catch_label is None:
            raise ValueError("try_catch() missing required keyword argument: 'catch_label'")
        attrs: dict[str, Any] = {
            graph_keys.TRY_LABEL: try_label,
            graph_keys.CATCH_LABEL: catch_label,
        }
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_ERR)
        if error_handler is None:
            raise ValueError("err() missing required keyword argument: 'error_handler'")
        attrs: dict[str, Any] = {graph_keys.RECOVERY_TEMPLATE: error_handler}
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_ERR, attrs)

    def communicate(
        self,
        *,
        name: str | None = None,
        target_agent: str | None = None,
        message: str | None = None,
        protocol: str | None = None,
        _template_scope: Mapping[str, Any] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Send a message to another agent (COMMUNICATE)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_COMMUNICATE)
        if target_agent is None:
            raise ValueError("communicate() missing required keyword argument: 'target_agent'")
        if message is None:
            raise ValueError("communicate() missing required keyword argument: 'message'")

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_message, auto_pairs = self._resolve_template_refs(
            message,
            template_scope=_template_scope,
        )

        attrs: dict[str, Any] = {
            graph_keys.RECIPIENT: target_agent,
            graph_keys.MESSAGE: resolved_message,
        }
        if auto_pairs:
            attrs[graph_keys.INPUT_NAMES] = [n for n, _ in auto_pairs]
        if protocol is not None:
            attrs[graph_keys.PROTOCOL] = protocol
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_COMMUNICATE, attrs)

        # Create auto-wire edges (in input_names order)
        for _name, ref in auto_pairs:
            self.add_edge(ref, node)

        session_node = self._agent_session_nodes.get(target_agent)
        if session_node is not None:
            self.add_edge(session_node, node, dependency=graph_keys.DEPENDENCY_DATA)
            self._agent_session_nodes[target_agent] = node

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
            name = self._auto_name(graph_keys.OP_UPDATE_GOAL)
        if goal_id is None:
            raise ValueError("update_goal() missing required keyword argument: 'goal_id'")
        attrs: dict[str, Any] = {
            graph_keys.GOAL_ID: goal_id,
            graph_keys.ACTION: action,
        }
        if priority is not None:
            attrs[graph_keys.PRIORITY] = priority
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_GUARD)
        if condition is None:
            raise ValueError("guard() missing required keyword argument: 'condition'")
        attrs: dict[str, Any] = {
            graph_keys.CONDITION: condition,
            graph_keys.ON_FAIL: on_fail,
        }
        if error_message is not None:
            attrs[graph_keys.ERROR_MESSAGE] = error_message
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_GUARD, attrs)
        if source is not None:
            self.add_edge(source, node)
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
            name = self._auto_name(graph_keys.OP_CLAIM)
        if queue is None:
            raise ValueError("claim() missing required keyword argument: 'queue'")
        attrs: dict[str, Any] = {graph_keys.QUEUE: queue}
        if lease_ms is not None:
            attrs[graph_keys.LEASE_MS] = lease_ms
        if max_wait_ms is not None:
            attrs[graph_keys.MAX_WAIT_MS] = max_wait_ms
        if server_url is not None:
            attrs[graph_keys.SERVER_URL] = server_url
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_PAUSE)
        if message is None:
            raise ValueError("pause() missing required keyword argument: 'message'")
        attrs: dict[str, Any] = {graph_keys.MESSAGE: message}
        if checkpoint_id is not None:
            attrs[graph_keys.CHECKPOINT_ID] = checkpoint_id
        if timeout_ms is not None:
            attrs[graph_keys.TIMEOUT_MS] = timeout_ms
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_RESUME)
        if checkpoint is None:
            raise ValueError("resume() missing required keyword argument: 'checkpoint'")
        attrs: dict[str, Any] = {graph_keys.CHECKPOINT: checkpoint}
        if poll_max_attempts is not None:
            attrs[graph_keys.POLL_MAX_ATTEMPTS] = poll_max_attempts
        if poll_interval_ms is not None:
            attrs[graph_keys.POLL_INTERVAL_MS] = poll_interval_ms
        if server_url is not None:
            attrs[graph_keys.SERVER_URL] = server_url
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_AGENT)
        attrs: dict[str, Any] = {}
        if memory is not None:
            attrs[graph_keys.MEMORY] = _normalize_value(memory)
        if beliefs is not None:
            attrs[graph_keys.BELIEFS] = _normalize_value(beliefs)
        if goals is not None:
            attrs[graph_keys.GOALS] = _normalize_value(goals)
        if capabilities is not None:
            attrs[graph_keys.CAPABILITIES] = _normalize_value(capabilities)
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_AGENT, attrs)

    def yield_(self, name: str | None = None, *, source: NodeRef | None = None, **attributes: Any) -> NodeRef:
        """Yield value from a switch-case region, compiler internal (YIELD)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_YIELD)
        node = self._add_node(name, graph_keys.OP_YIELD, self._apply_policy({}, attributes))
        if source is not None:
            self.add_edge(source, node)
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
            name = self._auto_name(graph_keys.OP_DELEGATE)
        if task_spec is None:
            raise ValueError("delegate() missing required keyword argument: 'task_spec'")
        if target_agent is None:
            raise ValueError("delegate() missing required keyword argument: 'target_agent'")
        attrs: dict[str, Any] = {
            graph_keys.TASK_SPEC: task_spec,
            graph_keys.TARGET_AGENT: target_agent,
        }
        attrs = self._apply_policy(attrs, attributes)
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
            name = self._auto_name(graph_keys.OP_NEGOTIATE)
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
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_NEGOTIATE, attrs)

    def nop(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """No-op passthrough with no side effects or AAM transition (NOP)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_NOP)
        return self._add_node(name, graph_keys.OP_NOP, self._apply_policy({}, attributes))

    def identity(self, name: str | None = None, **attributes: Any) -> NodeRef:
        """Identity passthrough that records an AAM identity transition (IDENTITY)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_IDENTITY)
        return self._add_node(name, graph_keys.OP_IDENTITY, self._apply_policy({}, attributes))

    def spawn_agent(
        self,
        name: str | None = None,
        *,
        agent_name: str | None = None,
        profile: AgentRef | None = None,
        mode: str | None = None,
        model: ModelId | None = None,
        cwd: str | None = None,
        capabilities: list[str] | None = None,
        goals: list[str] | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Create a new agent instance at runtime (SPAWN_AGENT).

        Args:
            name: Node name (auto-generated if not provided)
            agent_name: Name of the agent instance
            profile: Agent profile from apxm._generated.agents
            mode: Agent mode (e.g., "ask", "explore")
            model: Model name
            cwd: Working directory for the agent
            capabilities: List of capabilities
            goals: List of goals
        """
        if name is None:
            name = self._auto_name(graph_keys.OP_SPAWN_AGENT)
        if agent_name is None:
            raise ValueError("spawn_agent() missing required keyword argument: 'agent_name'")
        attrs: dict[str, Any] = {graph_keys.AGENT_NAME: agent_name}
        if profile is not None:
            attrs[graph_keys.PROFILE] = _agent_profile_name(profile)
        if mode is not None:
            attrs[graph_keys.MODE] = mode
        if model is not None:
            attrs[graph_keys.MODEL] = _normalize_model_id(model)
        if cwd is not None:
            attrs[graph_keys.CWD] = cwd
        if capabilities is not None:
            attrs[graph_keys.CAPABILITIES] = _normalize_value(capabilities)
        if goals is not None:
            attrs[graph_keys.GOALS] = _normalize_value(goals)
        attrs = self._apply_policy(attrs, attributes)
        node = self._add_node(name, graph_keys.OP_SPAWN_AGENT, attrs)
        self._agent_session_nodes[agent_name] = node
        return node

    def spawn_team(
        self,
        name: str | None = None,
        *,
        team_name: str | None = None,
        cwd: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Spawn a named team of agents (SPAWN_TEAM).

        The team roster is resolved by the runtime from ``~/.apxm/teams.toml``
        keyed by ``team_name``; members are not enumerated here. For ad-hoc,
        workflow-local teams use ``g.team(...).add(...)`` (the N-spawn_agent
        sugar) instead.
        """
        if name is None:
            name = self._auto_name(graph_keys.OP_SPAWN_TEAM)
        if team_name is None:
            raise ValueError("spawn_team() missing required keyword argument: 'team_name'")
        attrs: dict[str, Any] = {graph_keys.TEAM_NAME: team_name}
        if cwd is not None:
            attrs[graph_keys.CWD] = cwd
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_SPAWN_TEAM, attrs)

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
            name = self._auto_name(graph_keys.OP_REGISTER_CAPABILITY)
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
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_REGISTER_CAPABILITY, attrs)

    def autonomous(
        self,
        name: str | None = None,
        *,
        prompt: str | None = None,
        max_iterations: int | None = None,
        agent: AgentConfig | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | None = None,
        route: BackendRoute | None = None,
        backend: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Run a goal-directed autonomous loop (AUTONOMOUS)."""
        if name is None:
            name = self._auto_name(graph_keys.OP_AUTONOMOUS)
        if prompt is None:
            raise ValueError("autonomous() missing required argument: prompt")
        attrs: dict[str, Any] = {graph_keys.PROMPT: prompt}
        if max_iterations is not None:
            attrs[graph_keys.MAX_ITERATIONS] = max_iterations
        _apply_routing_attrs(attrs, route=route, model=model, provider=provider, backend=backend)
        attrs.update(_compose_system_prompt(agent, graph_keys.OP_AUTONOMOUS))
        attrs = self._apply_policy(attrs, attributes)
        return self._add_node(name, graph_keys.OP_AUTONOMOUS, attrs)

    def recv(
        self,
        name: str | None = None,
        *,
        recv_url: str,
        persona: str | None = None,
        once: bool = True,
        max_iterations: int | None = None,
        poll_interval_ms: int | None = None,
        recv_max_polls: int | None = None,
        agent: AgentConfig | None = None,
        model: ModelId | None = None,
        provider: ProviderSpec | None = None,
        route: BackendRoute | None = None,
        backend: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Park as an in-graph node until an event arrives at ``recv_url``, then
        run one agent turn per event (AUTONOMOUS ``mode=recv``).

        This is the artifact-resident counterpart to a host-loop monitor: the
        wait lives in the ``.air`` graph. ``once=True`` (default) returns after
        the first event; ``once=False`` re-arms up to ``max_iterations`` events.
        ``recv_url`` is polled (2xx + non-empty body = an event) every
        ``poll_interval_ms``; without ``recv_max_polls`` it waits indefinitely.
        """
        recv_attrs: dict[str, Any] = {"mode": "recv", "recv_url": recv_url}
        if not once:
            recv_attrs["recv_once"] = "false"
        if poll_interval_ms is not None:
            recv_attrs[graph_keys.POLL_INTERVAL_MS] = poll_interval_ms
        if recv_max_polls is not None:
            recv_attrs["recv_max_polls"] = recv_max_polls
        recv_attrs.update(attributes)
        return self.autonomous(
            name=name,
            prompt=persona or "You are an agent reacting to external events.",
            max_iterations=max_iterations,
            agent=agent,
            model=model,
            provider=provider,
            route=route,
            backend=backend,
            **recv_attrs,
        )

    def to_graph(self) -> ApxmGraph:
        from .backends import validate_graph_routes

        validate_graph_routes(self._nodes)
        return ApxmGraph(
            name=self._name,
            nodes=list(self._nodes),
            edges=list(self._edges),
            parameters=list(self._parameters),
            metadata=dict(self._metadata),
        )

    def register_python_tool(self, tool: Any) -> None:
        """Register a Python tool descriptor for artifact sidecar embedding.

        Called by Agent.ask() / BoundAgent.__init__() for each FunctionTool.
        Deduplicates by handler_id.
        """
        hid = tool.handler_id
        if hid in self._python_tool_ids:
            return
        self._python_tool_ids.add(hid)
        module = getattr(tool.fn, "__module__", "__unknown__") or "__unknown__"
        qualname = getattr(tool.fn, "__qualname__", tool.fn.__name__)
        descriptor = {
            graph_keys.PYTHON_TOOL_MANIFEST_HANDLER_ID: hid,
            graph_keys.PYTHON_TOOL_MANIFEST_MODULE: module,
            graph_keys.PYTHON_TOOL_MANIFEST_QUALNAME: qualname,
            graph_keys.PYTHON_TOOL_MANIFEST_NAME: tool.name,
            graph_keys.PYTHON_TOOL_MANIFEST_DESCRIPTION: tool.description,
            graph_keys.PYTHON_TOOL_MANIFEST_SCHEMA: json.loads(tool.schema_json) if tool.schema_json else {},
        }
        source_file = inspect.getsourcefile(tool.fn)
        if source_file:
            descriptor[graph_keys.PYTHON_TOOL_MANIFEST_SOURCE_FILE] = source_file
        self._python_tools.append(descriptor)

    def to_air(self) -> str:
        """Emit canonical .air text IR for this graph.

        Includes a sidecar comment for python_tools if any tools were registered.
        """
        air = self.to_graph().to_air()
        if self._python_tools:
            manifest = json.dumps(self._python_tools, separators=(",", ":"))
            air = f"{graph_keys.PYTHON_TOOLS_AIR_COMMENT_PREFIX}{manifest}\n{air}"
        return air

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


def _agent_profile_name(profile: Any) -> str:
    from ._generated.agents import AgentRef

    if isinstance(profile, AgentRef):
        return profile.name
    raise TypeError(
        "agent profile must be an AgentRef imported from apxm._generated.agents"
    )


def _apply_routing_attrs(
    attrs: dict[str, Any],
    *,
    route: Any | None,
    model: Any | None,
    provider: Any | None,
    backend: str | None,
) -> None:
    if backend is not None:
        raise TypeError("pass route=select_backend(...) instead of raw backend=")
    if route is not None and model is not None:
        raise ValueError("pass either route= or model=, not both")

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


def _coerce_node_policy(value: NodePolicy | dict[str, Any] | None) -> NodePolicy | None:
    if value is None:
        return None
    if isinstance(value, NodePolicy):
        return value
    if isinstance(value, dict):
        return NodePolicy(**value)
    raise TypeError(
        "policy must be a NodePolicy or dict[str, Any], "
        f"got {type(value).__name__}"
    )


def _extract_inline_node_policy(attributes: dict[str, Any]) -> NodePolicy | None:
    if "policy" in attributes and "node_policy" in attributes:
        raise ValueError("use either policy=... or node_policy=..., not both")

    raw_policy = attributes.pop("policy", None)
    if raw_policy is None:
        raw_policy = attributes.pop("node_policy", None)
    return _coerce_node_policy(raw_policy)
