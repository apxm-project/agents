/**
 * Graph IR mirroring apxm.ir (GraphNode/GraphEdge/Parameter/ApxmGraph) from
 * the Python frontend, plus the `to_air()` MLIR emitter.
 */
import { emitOp, isVoidOp, quote } from "./air-emit.js";
import type { OpName } from "./generated/ops.js";
import { topologicalSort } from "./utils.js";
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

/**
 * Attribute names that may carry `{name}` placeholders — mirrors Python's
 * `apxm/_generated/emission.py::TEMPLATE_ATTRS`. Used to rewrite `{param}` to
 * `{{param}}` for names that are flow parameters (see `_rewriteParamTemplates`).
 */
const TEMPLATE_ATTRS: ReadonlySet<string> = new Set([
  "template_str",
  "prompt",
  "template",
  "message",
  "params_json",
  "task_spec",
  "goal",
  "claim",
  "evidence",
  "trace_id",
  "discriminant",
  "recovery_template",
]);

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

  /** Emit a full `module { func.func @name(...) { ... } }` MLIR text block. */
  toAir(): string {
    const body = this._emitBodyLines();
    return this._wrapFuncLines(body);
  }

  /** Emit just the `func.func @name(...) { ... }` block, no `module` wrapper. */
  toFuncAir(): string {
    const air = this.toAir();
    const lines = air.split("\n");
    let body = lines;
    if (body.length > 0 && body[0].trim() === "module {") body = body.slice(1);
    if (body.length > 0 && body[body.length - 1].trim() === "}") body = body.slice(0, -1);
    return body.join("\n");
  }

  private _emitBodyLines(): string[] {
    const nodeIds = this.nodes.map((n) => n.id);
    const edgePairs: [number, number][] = this.edges.map((e) => [e.from, e.to]);

    let order: number[];
    try {
      order = topologicalSort(nodeIds, edgePairs);
    } catch {
      order = nodeIds;
    }

    const incoming = new Map<number, number[]>();
    const outgoing = new Map<number, number[]>();
    for (const id of nodeIds) {
      incoming.set(id, []);
      outgoing.set(id, []);
    }
    for (const edge of this.edges) {
      if (incoming.has(edge.from) && incoming.has(edge.to)) {
        if (edge.dependency === "Data") {
          incoming.get(edge.to)!.push(edge.from);
        }
        outgoing.get(edge.from)!.push(edge.to);
      }
    }

    const nodesById = new Map(this.nodes.map((n) => [n.id, n]));
    const produced = new Map<number, string>();
    const lines: string[] = [];

    // Function arguments (parameters) are referenced by templates as
    // `{name}`, rewritten below to `{{name}}` so the runtime substitutes them
    // at scheduler init. Compile params are not wired as Data inputs. Mirrors
    // `apxm.ir.ApxmGraph.to_air`'s param-name rewrite.
    const paramNames = new Set(this.parameters.map((p) => p.name));

    for (const nodeId of order) {
      const node = nodesById.get(nodeId);
      if (!node) continue;
      const ssaName = `%${node.name}`;
      const inputs = (incoming.get(nodeId) ?? [])
        .filter((srcId) => produced.has(srcId))
        .map((srcId) => produced.get(srcId)!);

      if (paramNames.size > 0) {
        const inputNameSet = new Set((node.attributes.input_names as string[] | undefined) ?? []);
        for (const attrName of Object.keys(node.attributes)) {
          if (!TEMPLATE_ATTRS.has(attrName)) continue;
          const text = node.attributes[attrName];
          if (typeof text !== "string") continue;
          (node.attributes as Record<string, unknown>)[attrName] = text.replace(
            /\{(\w+)\}/g,
            (whole, name: string) => (paramNames.has(name) && !inputNameSet.has(name) ? `{{${name}}}` : whole),
          );
        }
      }

      if (node.op === "RETURN") {
        if (inputs.length > 0) produced.set(nodeId, inputs[0]);
        continue;
      }

      const mlirLine = emitOp(node.op, ssaName, node.attributes, inputs);
      if (mlirLine) lines.push(`    ${mlirLine}`);

      if (!isVoidOp(node.op)) {
        produced.set(nodeId, ssaName);
      } else if (inputs.length > 0) {
        produced.set(nodeId, inputs[0]);
      }
    }

    const exitNodes = nodeIds.filter((id) => (outgoing.get(id) ?? []).length === 0);
    let returnVals = exitNodes.filter((id) => produced.has(id)).map((id) => produced.get(id)!);

    if (returnVals.length === 0) {
      for (let i = order.length - 1; i >= 0; i -= 1) {
        const id = order[i];
        if (produced.has(id)) {
          returnVals = [produced.get(id)!];
          break;
        }
      }
    }

    if (returnVals.length > 0) {
      if (returnVals.length === 1) {
        lines.push(`    func.return ${returnVals[0]} : !ais.token`);
      } else {
        const merged = "%ret_merge";
        const operands = returnVals.join(", ");
        const types = returnVals.map(() => "!ais.token").join(", ");
        lines.push(`    ${merged} = ais.merge ${operands} : ${types} -> !ais.token`);
        lines.push(`    func.return ${merged} : !ais.token`);
      }
    } else {
      lines.push(`    %result = ais.const_str ${quote("result")} : !ais.token`);
      lines.push("    func.return %result : !ais.token");
    }

    return lines;
  }

  private _wrapFuncLines(bodyLines: string[]): string {
    const funcName = sanitizeFlowName(this.name);
    const args = this.parameters.map(
      (param, i) => `%arg${i}: !ais.token {ais.param_name = "${param.name}", ais.param_type = "${param.typeName}"}`,
    );
    const argsStr = args.join(", ");

    let isEntry = this.metadata.is_entry ?? true;
    if (typeof isEntry === "string") {
      isEntry = ["true", "1", "yes"].includes(isEntry.toLowerCase());
    } else {
      isEntry = Boolean(isEntry);
    }
    const attrsStr = isEntry ? " attributes {ais.entry}" : "";

    const mlirLines = ["module {"];
    mlirLines.push(`  func.func @${funcName}(${argsStr}) -> !ais.token${attrsStr} {`);
    mlirLines.push(...bodyLines);
    mlirLines.push("  }");
    mlirLines.push("}");
    return mlirLines.join("\n");
  }
}

/** Sanitize a graph name for use as an MLIR function symbol (keeps dots). */
export function sanitizeFlowName(name: string): string {
  let sanitized = name.replace(/[^a-zA-Z0-9_.]/g, "_");
  if (sanitized.length > 0 && (/[0-9]/.test(sanitized[0]) || sanitized[0] === ".")) {
    sanitized = `flow_${sanitized}`;
  }
  return sanitized || "unnamed_flow";
}

/** Serialize multiple captured graphs into one `module { func.func @A ... }`. */
export function emitMultiFlowModule(graphs: readonly ApxmGraph[]): string {
  if (graphs.length === 0) return "module {\n}";
  const blocks = graphs.map((g) => g.toFuncAir());
  return `module {\n${blocks.join("\n")}\n}`;
}
