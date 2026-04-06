from __future__ import annotations

from .ir import ApxmGraph, GraphEdge, Parameter
from .proxy import GraphRecorder, NodeRef


class FlowModule:
    """Base class for declarative workflows. Analogous to nn.Module."""

    def define(self, g: GraphRecorder) -> NodeRef:
        """Override to define the graph. Analogous to forward()."""
        raise NotImplementedError

    def embed(self, g: GraphRecorder, prefix: str | None = None) -> NodeRef:
        """Capture this module into ``g`` and return the embedded terminal node."""
        child = GraphRecorder(type(self).__name__)
        terminal = self.define(child)
        if not isinstance(terminal, NodeRef):
            raise TypeError("FlowModule.define() must return a NodeRef")

        child_graph = child.to_graph()
        node_id_map: dict[int, NodeRef] = {}

        for node in child_graph.nodes:
            merged_name = node.name if prefix is None else f"{prefix}_{node.name}"
            merged_ref = g._add_node(merged_name, node.op, dict(node.attributes))
            node_id_map[node.id] = merged_ref

        for edge in child_graph.edges:
            g._edges.append(
                GraphEdge(
                    from_id=node_id_map[edge.from_id]._node_id,
                    to_id=node_id_map[edge.to_id]._node_id,
                    dependency=edge.dependency,
                )
            )

        for param in child_graph.parameters:
            merged_param = param.name if prefix is None else f"{prefix}_{param.name}"
            if any(existing.name == merged_param for existing in g._parameters):
                continue
            g._parameters.append(Parameter(name=merged_param, type_name=param.type_name))

        return node_id_map[terminal._node_id]

    def to_graph(self, name: str | None = None) -> ApxmGraph:
        recorder = GraphRecorder(name or type(self).__name__)
        self.define(recorder)
        return recorder.to_graph()
