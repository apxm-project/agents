from __future__ import annotations

from dataclasses import asdict, dataclass, field, is_dataclass
import json
from typing import Any

from apxm._generated import constants as c
from apxm._generated import operations


_DEPENDENCY_TYPES = {"Data", "Effect", "Control"}


def _normalize_air_value(value: Any) -> Any:
    if hasattr(value, "to_dict") and callable(value.to_dict):
        return _normalize_air_value(value.to_dict())
    if is_dataclass(value):
        return _normalize_air_value(asdict(value))
    if isinstance(value, tuple):
        return [_normalize_air_value(item) for item in value]
    if isinstance(value, list):
        return [_normalize_air_value(item) for item in value]
    if isinstance(value, dict):
        return {key: _normalize_air_value(item) for key, item in value.items()}
    return value


def _air_literal(value: Any) -> str:
    normalized = _normalize_air_value(value)
    try:
        return json.dumps(normalized, ensure_ascii=False, allow_nan=False)
    except (TypeError, ValueError):
        return json.dumps(str(normalized), ensure_ascii=False)


@dataclass(slots=True)
class GraphNode:
    id: int
    name: str
    op: str
    attributes: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "op": self.op,
            "attributes": dict(self.attributes),
        }

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "GraphNode":
        return cls(
            id=int(value["id"]),
            name=str(value["name"]),
            op=str(value["op"]),
            attributes=dict(value.get("attributes", {})),
        )


@dataclass(slots=True)
class GraphEdge:
    from_id: int
    to_id: int
    dependency: str = "Data"

    def __post_init__(self) -> None:
        if self.dependency not in _DEPENDENCY_TYPES:
            allowed = ", ".join(sorted(_DEPENDENCY_TYPES))
            raise ValueError(f"invalid dependency '{self.dependency}', expected one of: {allowed}")

    def to_dict(self) -> dict[str, Any]:
        return {
            "from": self.from_id,
            "to": self.to_id,
            "dependency": self.dependency,
        }

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "GraphEdge":
        return cls(
            from_id=int(value.get("from", value.get("from_id"))),
            to_id=int(value.get("to", value.get("to_id"))),
            dependency=str(value.get("dependency", "Data")),
        )


@dataclass(slots=True)
class Parameter:
    name: str
    type_name: str

    def to_dict(self) -> dict[str, Any]:
        return {"name": self.name, "type_name": self.type_name}

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "Parameter":
        return cls(name=str(value["name"]), type_name=str(value["type_name"]))


@dataclass(slots=True)
class ApxmGraph:
    name: str
    nodes: list[GraphNode] = field(default_factory=list)
    edges: list[GraphEdge] = field(default_factory=list)
    parameters: list[Parameter] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "nodes": [node.to_dict() for node in self.nodes],
            "edges": [edge.to_dict() for edge in self.edges],
            "parameters": [parameter.to_dict() for parameter in self.parameters],
            "metadata": dict(self.metadata),
        }

    def to_json(self, *, indent: int = 2, sort_keys: bool = False) -> str:
        return json.dumps(self.to_dict(), indent=indent, sort_keys=sort_keys)

    def to_air(self) -> str:
        """Emit valid MLIR text for this graph.

        The .air format now produces VALID MLIR that can be parsed directly by
        MLIR's Module::parse(). No custom parser needed.

        Example output:
            module {
              func.func @research_pipeline(%arg0: !ais.token {ais.param_name = "topic", ais.param_type = "str"}) -> !ais.token attributes {ais.entry} {
                %research = ais.ask "Research: {0}" [%arg0 : !ais.token] : !ais.token
                %critique = ais.ask "Critique: {0}" [%arg0 : !ais.token] : !ais.token
                %sync = ais.wait_all %research, %critique : !ais.token, !ais.token -> !ais.token
                func.return %sync : !ais.token
              }
            }
        """
        from .utils import topological_sort

        # Topological sort
        node_ids = [n.id for n in self.nodes]
        edges = [(e.from_id, e.to_id) for e in self.edges]
        try:
            order = topological_sort(node_ids, edges)
        except ValueError as e:
            # Fallback: use node order as-is if cycle detection fails
            order = node_ids

        # Build adjacency structures
        incoming: dict[int, list[int]] = {nid: [] for nid in node_ids}
        outgoing: dict[int, list[int]] = {nid: [] for nid in node_ids}
        for edge in self.edges:
            if edge.from_id in incoming and edge.to_id in incoming:
                # Only include Data edges as inputs (Control/Effect are for sequencing only)
                if edge.dependency == "Data":
                    incoming[edge.to_id].append(edge.from_id)
                outgoing[edge.from_id].append(edge.to_id)

        # Map node_id -> node
        nodes_by_id = {n.id: n for n in self.nodes}

        # Track produced SSA values
        produced: dict[int, str] = {}

        # Function arguments (parameters)
        arg_values = [f"%arg{i}" for i in range(len(self.parameters))]

        lines: list[str] = []
        emitted_func_return = False  # Track if we emitted func.return from a RETURN node

        # Emit nodes in topological order
        for node_id in order:
            node = nodes_by_id[node_id]
            ssa_name = f"%{node.name}"

            # Get inputs from incoming edges
            inputs = [produced[src_id] for src_id in incoming[node_id] if src_id in produced]

            # Inject parameters for nodes that have no inputs and use parameters
            if not inputs and arg_values and self._node_uses_params(node):
                inputs = arg_values.copy()

            # Emit the operation
            mlir_line = self._emit_mlir_op(node, ssa_name, inputs)
            if mlir_line:
                lines.append(f"    {mlir_line}")
                # Track if this was a func.return from a RETURN node
                if node.op.upper() == "RETURN" and "func.return" in mlir_line:
                    emitted_func_return = True

            # Track produced value
            if not self._is_void_op(node.op):
                produced[node_id] = ssa_name

        # Find exit nodes (no outgoing edges) and emit func.return
        exit_nodes = [nid for nid in node_ids if not outgoing[nid]]

        # Only add func.return if we didn't already emit one from a RETURN node
        if not emitted_func_return:
            # Find return value
            return_vals = [produced[nid] for nid in exit_nodes if nid in produced]
            if not return_vals:
                # No exit nodes with values, find last produced value
                for nid in reversed(order):
                    if nid in produced:
                        return_vals = [produced[nid]]
                        break

            if return_vals:
                if len(return_vals) == 1:
                    lines.append(f"    func.return {return_vals[0]} : !ais.token")
                else:
                    # Merge multiple return values
                    merged = "%ret_merge"
                    operands = ", ".join(return_vals)
                    types = ", ".join(["!ais.token"] * len(return_vals))
                    lines.append(f"    {merged} = ais.merge {operands} : {types} -> !ais.token")
                    lines.append(f"    func.return {merged} : !ais.token")
            else:
                # No values produced, create a const token
                lines.append("    %result = ais.const_str \"result\" : !ais.token")
                lines.append("    func.return %result : !ais.token")

        # Build function signature
        func_name = self._sanitize_name(self.name)
        args = []
        for i, param in enumerate(self.parameters):
            args.append(
                f"%arg{i}: !ais.token {{ais.param_name = \"{param.name}\", ais.param_type = \"{param.type_name}\"}}"
            )
        args_str = ", ".join(args)

        # Check if this is an entry flow
        is_entry = self.metadata.get("is_entry", True)
        if isinstance(is_entry, bool):
            pass
        elif isinstance(is_entry, str):
            is_entry = is_entry.lower() in ("true", "1", "yes")
        else:
            is_entry = bool(is_entry)

        attrs_str = " attributes {ais.entry}" if is_entry else ""

        # Assemble module
        mlir_lines = ["module {"]
        mlir_lines.append(f"  func.func @{func_name}({args_str}) -> !ais.token{attrs_str} {{")
        mlir_lines.extend(lines)
        mlir_lines.append("  }")
        mlir_lines.append("}")

        return "\n".join(mlir_lines)

    def _node_uses_params(self, node: GraphNode) -> bool:
        """Check if a node uses flow parameters in its template/prompt attributes."""
        if not self.parameters:
            return False

        template_attrs = ["template_str", "prompt", "template", "value"]
        for attr_name in template_attrs:
            if attr_name in node.attributes:
                text = str(node.attributes[attr_name])
                # Check for positional placeholders {0}, {1}, ..., {N-1}
                for i in range(len(self.parameters)):
                    if f"{{{i}}}" in text:
                        return True
                # Also check for named {{PARAM_NAME}} pattern
                for param in self.parameters:
                    if f"{{{{{param.name}}}}}" in text:
                        return True
        return False

    def _sanitize_name(self, name: str) -> str:
        """Sanitize graph name for use as MLIR function name."""
        # Replace spaces and special chars with underscores
        import re
        sanitized = re.sub(r'[^a-zA-Z0-9_]', '_', name)
        # Ensure it doesn't start with a digit
        if sanitized and sanitized[0].isdigit():
            sanitized = f"flow_{sanitized}"
        return sanitized or "unnamed_flow"

    def _is_void_op(self, op: str) -> bool:
        """Check if an operation produces no result (void)."""
        void_ops = {"UMEM", "PRINT", "FENCE", "JUMP", "TRY_CATCH", "PAUSE", "RETURN"}
        return op.upper() in void_ops

    def _emit_op_fallback(self, op: str, ssa_name: str, attrs: dict[str, Any], inputs: list[str]) -> str:
        """Fallback MLIR emission using hardcoded patterns.

        This function contains the original hardcoded emission logic and serves as
        a safety net when auto-generated emitters are not available.
        """
        # Helper to format context/inputs
        def fmt_context(bracket_style: str = "[]") -> str:
            if not inputs:
                return ""
            left, right = bracket_style[0], bracket_style[1]
            types = ", ".join(["!ais.token"] * len(inputs))
            return f" {left}{', '.join(inputs)} : {types}{right}"

        # Helper to get attribute with fallback
        def get_attr(keys: list[str], default: str = "") -> str:
            for key in keys:
                if key in attrs:
                    val = attrs[key]
                    return str(val) if val is not None else default
            return default

        # Helper to quote string for MLIR
        def quote(s: str) -> str:
            # Escape backslashes first, then quotes, then newlines and tabs
            escaped = s.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n').replace('\t', '\\t').replace('\r', '\\r')
            return f'"{escaped}"'

        # Emit based on operation type
        if op == "CONST_STR":
            value = get_attr(["value", "text", "const", "template_str", "prompt"], "")
            return f"{ssa_name} = ais.const_str {quote(value)} : !ais.token"

        elif op in ("ASK", "THINK", "REASON"):
            template = get_attr(["template_str", "prompt", "template"], "{0}")
            op_name = op.lower()
            ctx = fmt_context("[]")
            return f"{ssa_name} = ais.{op_name} {quote(template)}{ctx} : !ais.token"

        elif op == "QMEM":
            query = get_attr(["query"], "")
            sid = get_attr(["sid", "stage", "scope"], "default")
            space = get_attr(["space", "memory_tier"], "stm")
            limit = attrs.get("limit")
            limit_str = f" limit {limit}" if limit else ""
            return f"{ssa_name} = ais.qmem {quote(query)} stage {quote(sid)} in {space}{limit_str} : !ais.handle<{space}>"

        elif op == "UMEM":
            space = get_attr(["space", "memory_tier"], "stm")
            key = attrs.get("key")
            source = inputs[0] if inputs else "%const_mem"
            key_str = f" {{key = {quote(key)}}}" if key else ""
            return f"ais.umem {source} into {space}{key_str} : !ais.token"

        elif op == "INV_TOOL":
            capability = get_attr(["capability"], "unknown")
            params_json = get_attr(["params_json"], "{}")
            return f"{ssa_name} = ais.inv_tool {quote(capability)} ({quote(params_json)}) : !ais.token"

        elif op in ("WAIT_ALL", "MERGE"):
            op_name = op.lower()
            if inputs:
                operands = ", ".join(inputs)
                types = ", ".join(["!ais.token"] * len(inputs))
                return f"{ssa_name} = ais.{op_name} {operands} : {types} -> !ais.token"
            else:
                return f"{ssa_name} = ais.{op_name} -> !ais.token"

        elif op == "PRINT":
            message = get_attr(["message", "template_str", "prompt"], "{0}")
            ctx = fmt_context("[]")
            return f"ais.print {quote(message)}{ctx}"

        elif op == "SPAWN_AGENT":
            agent_name = get_attr(["agent_name", "name"], "agent")
            profile = attrs.get("profile")
            mode = attrs.get("mode")
            model = attrs.get("model")
            cwd = attrs.get("cwd")
            attr_parts = []
            if profile:
                attr_parts.append(f"profile = {quote(profile)}")
            if mode:
                attr_parts.append(f"mode = {quote(mode)}")
            if model:
                attr_parts.append(f"model = {quote(model)}")
            if cwd:
                attr_parts.append(f"cwd = {quote(cwd)}")
            attr_str = f" {{{', '.join(attr_parts)}}}" if attr_parts else ""
            return f"{ssa_name} = ais.spawn_agent {quote(agent_name)}{attr_str} : !ais.token"

        elif op == "COMMUNICATE":
            message = get_attr(["message", "payload", "template_str", "prompt"], "{0}")
            recipient = get_attr(["recipient", "target"], "default")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.communicate {quote(message)} to {quote(recipient)}{ctx} : !ais.token"

        elif op == "FENCE":
            return "ais.fence"

        elif op == "RETURN":
            # RETURN nodes with inputs become func.return
            # RETURN nodes without inputs are markers and should be handled
            # by the automatic return logic (they just mark the exit point)
            if inputs:
                return f"func.return {inputs[0]} : !ais.token"
            else:
                # No explicit return value - let the automatic logic handle it
                # Don't emit anything here, the check for has_return_node will be false
                return ""  # Empty string means no output for this node

        elif op == "PLAN":
            goal = get_attr(["goal"], "goal")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.plan {quote(goal)}{ctx} : !ais.goal<0>"

        elif op == "REFLECT":
            trace_id = get_attr(["trace_id", "trace"], "")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.reflect {quote(trace_id)}{ctx} : !ais.token"

        elif op == "VERIFY":
            template = get_attr(["template_str", "prompt"], "Verify")
            claim = inputs[0] if len(inputs) > 0 else "%claim"
            evidence = inputs[1] if len(inputs) > 1 else claim
            return f"{ssa_name} = ais.verify {claim} : !ais.token vs {evidence} : !ais.token with {quote(template)} : !ais.token"

        elif op == "EXC":
            code = get_attr(["code", "script"], "")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.exc {quote(code)}{ctx} : !ais.token"

        elif op == "JUMP":
            target = get_attr(["target", "label"], "next")
            return f"ais.jump {quote(target)}"

        elif op == "BRANCH_ON_VALUE":
            condition = inputs[0] if inputs else "%cond"
            true_label = get_attr(["true_label"], "true")
            false_label = get_attr(["false_label"], "false")
            return f"ais.branch_on_value {condition}, {quote(true_label)}, {quote(false_label)} : !ais.token"

        elif op == "LOOP_START":
            count = inputs[0] if inputs else "%count"
            label = get_attr(["label"], "loop")
            return f"{ssa_name} = ais.loop_start {count} as {quote(label)} : !ais.token -> !ais.token"

        elif op == "LOOP_END":
            state = inputs[0] if inputs else "%state"
            return f"{ssa_name} = ais.loop_end {state} : !ais.token -> !ais.token"

        elif op == "TRY_CATCH":
            try_label = get_attr(["try_label"], "try")
            catch_label = get_attr(["catch_label"], "catch")
            return f"ais.try_catch {quote(try_label)} -> {quote(catch_label)}"

        elif op == "ERR":
            recovery = get_attr(["recovery_template"], "Recover")
            if inputs:
                return f"{ssa_name} = ais.err {inputs[0]} : !ais.token with {quote(recovery)} -> !ais.token"
            else:
                return f"{ssa_name} = ais.err with {quote(recovery)} -> !ais.token"

        elif op == "CHECKPOINT":
            checkpoint_id = get_attr(["checkpoint_id", "checkpoint"], "checkpoint")
            ctx = fmt_context("[]")
            return f"{ssa_name} = ais.checkpoint {quote(checkpoint_id)}{ctx} : !ais.token"

        elif op == "UPDATE_GOAL":
            goal_id = get_attr(["goal_id", "goal"], "goal")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.update_goal {quote(goal_id)}{ctx} : !ais.token"

        elif op == "GUARD":
            condition = get_attr(["condition", "template_str"], "true")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.guard {quote(condition)}{ctx} : !ais.token"

        elif op == "CLAIM":
            queue = get_attr(["queue", "key"], "default")
            lease_ms = attrs.get("lease_ms")
            lease_str = f" {{lease_ms = {lease_ms} : i64}}" if lease_ms else ""
            return f"{ssa_name} = ais.claim {quote(queue)}{lease_str} : !ais.token"

        elif op == "PAUSE":
            message = get_attr(["message", "checkpoint"], "paused")
            ctx = fmt_context("[]")
            return f"ais.pause {quote(message)}{ctx}"

        elif op == "RESUME":
            checkpoint = get_attr(["checkpoint", "checkpoint_id"], "latest")
            return f"{ssa_name} = ais.resume {quote(checkpoint)} : !ais.token"

        elif op == "SPAWN_TEAM":
            team_name = get_attr(["team_name", "name"], "team")
            profile = attrs.get("profile")
            attr_str = f" {{profile = {quote(profile)}}}" if profile else ""
            return f"{ssa_name} = ais.spawn_team {quote(team_name)}{attr_str} : !ais.token"

        elif op == "FLOW_CALL":
            agent_name = get_attr(["agent_name", "target"], "agent")
            flow_name = get_attr(["flow_name", "flow"], "flow")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.flow_call {quote(agent_name)} {quote(flow_name)}{ctx} : !ais.token"

        elif op == "DELEGATE":
            target = get_attr(["target_agent", "target"], "agent")
            task = get_attr(["task_spec", "template_str"], "{0}")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.delegate {quote(task)} to {quote(target)}{ctx} : !ais.token"

        elif op == "NEGOTIATE":
            topic = get_attr(["proposal", "template_str", "topic"], "{0}")
            ctx = fmt_context("()")
            return f"{ssa_name} = ais.negotiate {quote(topic)}{ctx} : !ais.token"

        elif op == "REGISTER_CAPABILITY":
            cap_name = get_attr(["capability_name", "name"], "capability")
            desc = attrs.get("description")
            attr_str = f" {{description = {quote(desc)}}}" if desc else ""
            return f"{ssa_name} = ais.register_capability {quote(cap_name)}{attr_str} : !ais.token"

        elif op == "AUTONOMOUS":
            strategy = attrs.get("strategy")
            template = attrs.get("template_str")
            attr_parts = []
            if strategy:
                attr_parts.append(f"strategy = {quote(strategy)}")
            if template:
                attr_parts.append(f"template_str = {quote(template)}")
            attr_str = f" {{{', '.join(attr_parts)}}}" if attr_parts else ""
            return f"{ssa_name} = ais.autonomous{attr_str} : !ais.token"

        elif op in ("NOP", "IDENTITY", "YIELD"):
            op_name = op.lower()
            if inputs:
                # Passthrough
                return f"{ssa_name} = {inputs[0]}"  # Not a real op, just alias
            else:
                return f"{ssa_name} = ais.{op_name} : !ais.token"

        else:
            # Fallback: generic op
            return f"{ssa_name} = ais.{op.lower()} : !ais.token"

    def _emit_op(self, op: str, ssa_name: str, attrs: dict[str, Any], inputs: list[str]) -> str:
        """Dispatch to auto-generated emitters if available, otherwise use fallback.

        This function tries to import and use auto-generated emitters from
        apxm._generated.emission first. If that module doesn't exist or the
        specific op emitter is not available, it falls back to the hardcoded
        emission logic in _emit_op_fallback.
        """
        try:
            from apxm._generated.emission import EMITTERS
            if op in EMITTERS:
                return EMITTERS[op](ssa_name, attrs, inputs)
        except (ImportError, KeyError, AttributeError):
            pass

        # Fallback to hardcoded emission
        return self._emit_op_fallback(op, ssa_name, attrs, inputs)

    def _emit_mlir_op(self, node: GraphNode, ssa_name: str, inputs: list[str]) -> str:
        """Emit MLIR assembly for a single operation.

        Delegates to _emit_op which tries auto-generated emitters first,
        then falls back to hardcoded patterns.
        """
        return self._emit_op(node.op.upper(), ssa_name, node.attributes, inputs)

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "ApxmGraph":
        return cls(
            name=str(value["name"]),
            nodes=[GraphNode.from_dict(node) for node in value.get("nodes", [])],
            edges=[GraphEdge.from_dict(edge) for edge in value.get("edges", [])],
            parameters=[
                Parameter.from_dict(parameter) for parameter in value.get("parameters", [])
            ],
            metadata=dict(value.get("metadata", {})),
        )

    @classmethod
    def from_json(cls, payload: str | bytes | bytearray) -> "ApxmGraph":
        if isinstance(payload, (bytes, bytearray)):
            payload = payload.decode("utf-8")
        return cls.from_dict(json.loads(payload))

    @classmethod
    def merge(cls, name: str, graphs: list["ApxmGraph"]) -> "ApxmGraph":
        """Merge multiple sub-graphs, remapping node IDs to avoid collisions.

        Adds a WAIT_ALL synchronization node collecting all sub-graph exit nodes.
        Parameters are deduplicated by name (first wins).
        """
        if not graphs:
            return cls(name=name)

        merged_nodes: list[GraphNode] = []
        merged_edges: list[GraphEdge] = []
        merged_params: list[Parameter] = []
        merged_metadata: dict[str, Any] = {}
        seen_param_names: set[str] = set()

        id_offset = 0

        for graph in graphs:
            # Build remap table
            remap: dict[int, int] = {node.id: node.id + id_offset for node in graph.nodes}

            # Add remapped nodes
            for node in graph.nodes:
                merged_nodes.append(
                    GraphNode(
                        id=remap[node.id],
                        name=node.name,
                        op=node.op,
                        attributes=dict(node.attributes),
                    )
                )

            # Add remapped edges
            for edge in graph.edges:
                merged_edges.append(
                    GraphEdge(
                        from_id=remap[edge.from_id],
                        to_id=remap[edge.to_id],
                        dependency=edge.dependency,
                    )
                )

            # Deduplicate parameters (first wins)
            for param in graph.parameters:
                if param.name not in seen_param_names:
                    seen_param_names.add(param.name)
                    merged_params.append(Parameter(name=param.name, type_name=param.type_name))

            # Merge metadata (first wins)
            for k, v in graph.metadata.items():
                if k not in merged_metadata:
                    merged_metadata[k] = v

            # Advance offset past max ID in this sub-graph
            if graph.nodes:
                id_offset += max(node.id for node in graph.nodes)

        # Find exit nodes (no outgoing edges)
        sources = {edge.from_id for edge in merged_edges}
        exit_ids = [node.id for node in merged_nodes if node.id not in sources]

        # Create WAIT_ALL sync node
        wait_all_id = max(node.id for node in merged_nodes) + 1
        merged_nodes.append(
            GraphNode(id=wait_all_id, name=f"{name}_sync", op=operations.WAIT_ALL.op)
        )

        # Control edges from exit nodes to sync
        for exit_id in exit_ids:
            merged_edges.append(
                GraphEdge(from_id=exit_id, to_id=wait_all_id, dependency="Control")
            )

        return cls(
            name=name,
            nodes=merged_nodes,
            edges=merged_edges,
            parameters=merged_params,
            metadata=merged_metadata,
        )


# ============================================================================
# APXM contract validation
# ============================================================================

# Required attributes per operation, derived from Rust AIS_OPERATIONS when
# the native module is available.  Falls back to a hardcoded dict aligned
# with the JSON wire-format keys from apxm-core/src/constants.rs.
# Note: some OperationSpec field names in definitions.rs differ from the
# wire-format keys (e.g. "label_true" vs "true_label") because the MLIR
# compiler performs attribute-name translation at emission time.  Ops whose
# required inputs come solely from graph edges (MERGE, WAIT_ALL, RETURN)
# have empty sets here — the compiler resolves them from edge topology.
try:
    from apxm._native import get_ais_operations as _get_ais_operations

    _REQUIRED_ATTRS: dict[str, set[str]] = {
        spec["op"]: set(spec["required_fields"])
        for spec in _get_ais_operations()
    }
except ImportError:
    # Fallback: use generated operation specs
    _REQUIRED_ATTRS: dict[str, set[str]] = {
        spec.op: set(spec.required_fields)
        for spec in operations.ALL_OPERATIONS
    }

# Valid parameter type_name values accepted by the APXM runtime.
_VALID_PARAM_TYPES: set[str] = {"str", "int", "float", "bool", "json"}


@dataclass(slots=True)
class ValidationResult:
    """Result of validating an ApxmGraph against the APXM contract."""

    valid: bool
    errors: list[str]
    warnings: list[str]


def validate_against_apxm(graph: ApxmGraph) -> ValidationResult:
    """Validate an ``ApxmGraph`` against the APXM compiler contract.

    Checks performed:

    * All operation names are valid AIS ops.
    * Required attributes present for each op type (warning, not error,
      because the Python SDK often sets slightly different key names).
    * All node IDs are unique and positive.
    * Edges reference existing node IDs.
    * Parameter ``type_name`` values are valid.
    * Graph name is non-empty.
    * No duplicate parameter names.
    * Graph edges form a DAG (no cycles).
    """
    errors: list[str] = []
    warnings: list[str] = []

    # --- graph-level checks ------------------------------------------------
    if not graph.name:
        errors.append("graph name must not be empty")

    if not graph.nodes:
        errors.append("graph must contain at least one node")

    # --- node checks -------------------------------------------------------
    node_ids: set[int] = set()
    for node in graph.nodes:
        # unique positive IDs
        if node.id <= 0:
            errors.append(f"node '{node.name}' has non-positive id {node.id}")
        if node.id in node_ids:
            errors.append(f"duplicate node id {node.id}")
        node_ids.add(node.id)

        # empty name
        if not node.name:
            errors.append(f"node id={node.id} has empty name")

        # valid op
        if node.op not in _REQUIRED_ATTRS:
            errors.append(
                f"node '{node.name}' (id={node.id}) has unknown op '{node.op}'"
            )
        else:
            # check required attributes (as warnings -- SDK may use aliases)
            required = _REQUIRED_ATTRS[node.op]
            present = set(node.attributes.keys())
            missing = required - present
            if missing:
                warnings.append(
                    f"node '{node.name}' ({node.op}): missing recommended "
                    f"attributes {sorted(missing)}"
                )

    # --- edge checks -------------------------------------------------------
    for edge in graph.edges:
        if edge.from_id not in node_ids:
            errors.append(
                f"edge references non-existent from_id {edge.from_id}"
            )
        if edge.to_id not in node_ids:
            errors.append(
                f"edge references non-existent to_id {edge.to_id}"
            )

    # --- parameter checks --------------------------------------------------
    param_names: set[str] = set()
    for param in graph.parameters:
        if not param.name:
            errors.append("parameter with empty name")
        if param.name in param_names:
            errors.append(f"duplicate parameter name '{param.name}'")
        param_names.add(param.name)
        if param.type_name not in _VALID_PARAM_TYPES:
            warnings.append(
                f"parameter '{param.name}' has non-standard type_name "
                f"'{param.type_name}'"
            )

    # --- DAG cycle check (Kahn's algorithm) --------------------------------
    if graph.nodes and graph.edges:
        from .utils import detect_cycle

        edges = [
            (edge.from_id, edge.to_id)
            for edge in graph.edges
            if edge.from_id in node_ids and edge.to_id in node_ids
        ]
        cycle_count = detect_cycle(node_ids, edges)
        if cycle_count:
            errors.append(
                f"graph contains a cycle ({cycle_count} nodes involved)"
            )

    return ValidationResult(
        valid=len(errors) == 0,
        errors=errors,
        warnings=warnings,
    )
