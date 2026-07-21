/**
 * Graph IR for the TypeScript frontend: nodes, edges, parameters, metadata.
 */
import type { OpName } from "./generated/ops.js";
import { type DependencyType, normalizeDependencyType, type ParamType } from "./types.js";

export interface GraphNode {
  readonly id: number;
  readonly name: string;
  readonly op: OpName;
  readonly attributes: Record<string, unknown>;
}

export interface GraphEdge {
  readonly from: number;
  readonly to: number;
  readonly dependency: DependencyType;
}

export function makeEdge(from: number, to: number, dependency: DependencyType | string = "Data"): GraphEdge {
  return { from, to, dependency: normalizeDependencyType(dependency as DependencyType) };
}

export interface Parameter {
  readonly name: string;
  readonly typeName: ParamType | string;
}

export interface ApxmGraphData {
  name: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  parameters: Parameter[];
  metadata: Record<string, unknown>;
}

/**
 * A recorded workflow graph: nodes (AIS ops), edges (Data/Control/Effect
 * dependencies), declared parameters, and free-form metadata. Analogous to
 * Python's `apxm.ir.ApxmGraph`.
 */
export class ApxmGraph implements ApxmGraphData {
  name: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  parameters: Parameter[];
  metadata: Record<string, unknown>;

  constructor(data: ApxmGraphData) {
    this.name = data.name;
    this.nodes = data.nodes;
    this.edges = data.edges;
    this.parameters = data.parameters;
    this.metadata = data.metadata;
  }

  toDict(): Record<string, unknown> {
    return {
      name: this.name,
      nodes: this.nodes.map((n) => ({ id: n.id, name: n.name, op: n.op, attributes: { ...n.attributes } })),
      edges: this.edges.map((e) => ({ from: e.from, to: e.to, dependency: e.dependency })),
      parameters: this.parameters.map((p) => ({ name: p.name, type_name: p.typeName })),
      metadata: { ...this.metadata },
    };
  }

}
