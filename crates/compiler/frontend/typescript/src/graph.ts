/**
 * Graph IR for the TypeScript frontend: nodes, edges, parameters, metadata.
 */
import type { OpName } from "./generated/ops.js";
import { type DependencyType, normalizeDependencyType, type ParamType } from "./types.js";

type ProcessLike = {
  env?: Record<string, string | undefined>;
  getBuiltinModule?: (specifier: string) => unknown;
};

type SpawnSync = (
  command: string,
  args: readonly string[],
  options: { input: string; encoding: "utf8" },
) => { status: number | null; stdout: string; stderr: string; error?: Error };

class FrontendAirCommandError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "FrontendAirCommandError";
  }
}

class FrontendAirEmitter {
  private constructor(
    private readonly spawnSync: SpawnSync,
    private readonly command: string,
    private readonly args: readonly string[],
  ) {}

  static fromRuntime(processLike: ProcessLike | undefined): FrontendAirEmitter {
    const getBuiltinModule = processLike?.getBuiltinModule;
    if (!getBuiltinModule) {
      throw new FrontendAirCommandError("ApxmGraph.toAir requires Node.js process builtins");
    }
    const childProcess = getBuiltinModule("node:child_process") as { spawnSync: SpawnSync };
    // Single path to the one Rust printer: `apxm emit-air`. APXM_BIN overrides
    // which binary (the compile pipeline sets it to its own executable);
    // otherwise `apxm` must be on PATH.
    const apxmBin = processLike?.env?.APXM_BIN || "apxm";
    return new FrontendAirEmitter(childProcess.spawnSync, apxmBin, ["emit-air"]);
  }

  emit(input: unknown): string {
    const result = this.spawnSync(this.command, this.args, {
      input: JSON.stringify(input),
      encoding: "utf8",
    });
    if (result.error) {
      throw result.error;
    }
    if (result.status !== 0) {
      const detail = result.stderr.trim() || `exit status ${result.status}`;
      throw new FrontendAirCommandError(`${this.command} ${this.args.join(" ")} failed: ${detail}`);
    }
    return result.stdout;
  }
}

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

  /** Emit a full `module { func.func @name(...) { ... } }` AIR block. */
  toAir(): string {
    return emitAirFromFrontendGraph(this.toDict());
  }

}

/** Serialize multiple captured graphs into one AIR module. */
export function emitMultiFlowModule(graphs: readonly ApxmGraph[]): string {
  return emitAirFromFrontendGraph(graphs.map((graph) => graph.toDict()));
}

function emitAirFromFrontendGraph(input: unknown): string {
  const processLike = (globalThis as { process?: ProcessLike }).process;
  return FrontendAirEmitter.fromRuntime(processLike).emit(input);
}
