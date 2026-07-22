// GraphBuilder and native bridge helpers — no imports from agent-program.

import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const native = require(join(here, "_native.node")) as {
  compileFrontendGraphArtifact(json: string): string;
  lowerFrontendGraph(json: string): string;
  verifyFrontendGraph(json: string): string | null;
};

export const OP_MODEL_CALL = "model.call" as const;
export const OP_CAPABILITY_INVOKE = "capability.invoke" as const;
export const OP_PROGRAM_NEW = "program.new" as const;
export const OP_PROGRAM_INVOKE = "program.invoke" as const;
export const OP_AWAIT_EVENT = "await.event" as const;

export type OpName =
  | typeof OP_MODEL_CALL
  | typeof OP_CAPABILITY_INVOKE
  | typeof OP_PROGRAM_NEW
  | typeof OP_PROGRAM_INVOKE
  | typeof OP_AWAIT_EVENT;

export const FIVE_OPS: ReadonlySet<OpName> = new Set([
  OP_MODEL_CALL,
  OP_CAPABILITY_INVOKE,
  OP_PROGRAM_NEW,
  OP_PROGRAM_INVOKE,
  OP_AWAIT_EVENT,
]);

export const FRONTEND_GRAPH_VERSION = "apxm.frontend-graph.v1";
export const SOURCE_MAP_VERSION = "apxm.source-map.v1";

export type Json = Record<string, unknown>;

export class GraphBuilder {
  private graph: Json;

  constructor(sourceLanguage = "typescript") {
    this.graph = {
      schema_version: FRONTEND_GRAPH_VERSION,
      source_language: sourceLanguage,
      program_definitions: [],
      imported_program_refs: [],
      semantic_operations: [],
      structural_regions: [],
      context_flow: [],
      hook_bindings: [],
      capability_requirements: [],
      model_requirements: [],
      source_map: {
        schema_version: SOURCE_MAP_VERSION,
        source_language: sourceLanguage,
        node_spans: [],
        region_annotations: [],
      },
    };
  }

  program(def: {
    program_id: string;
    entrypoint: string;
    input_type_ref: string;
    output_type_ref: string;
    has_default_context: boolean;
    context_type_ref?: string;
  }): this {
    const record: Json = {
      program_id: def.program_id,
      entrypoint: def.entrypoint,
      input_type_ref: def.input_type_ref,
      output_type_ref: def.output_type_ref,
      has_default_context: def.has_default_context,
    };
    if (def.context_type_ref !== undefined) {
      record.context_type_ref = def.context_type_ref;
    }
    (this.graph.program_definitions as Json[]).push(record);
    return this;
  }

  importProgram(ref: {
    program_ref: string;
    artifact_digest: string;
    entrypoint: string;
    target_agent_identity_requirement: string;
  }): this {
    (this.graph.imported_program_refs as Json[]).push({ ...ref });
    return this;
  }

  private op(nodeId: string, op: string, operands?: Json): this {
    const record: Json = { node_id: nodeId, op };
    if (operands !== undefined) {
      record.operands = operands;
    }
    (this.graph.semantic_operations as Json[]).push(record);
    return this;
  }

  modelCall(nodeId: string, modelTargetRef?: string): this {
    return this.op(
      nodeId,
      OP_MODEL_CALL,
      modelTargetRef === undefined ? undefined : { model_target_ref: modelTargetRef },
    );
  }

  capabilityInvoke(nodeId: string, capabilityRef?: string): this {
    return this.op(
      nodeId,
      OP_CAPABILITY_INVOKE,
      capabilityRef === undefined ? undefined : { capability_ref: capabilityRef },
    );
  }

  externalAgentCapability(nodeId: string, profileRef: string, sessionRef: string): this {
    return this.op(nodeId, OP_CAPABILITY_INVOKE, {
      capability_ref: `external-agent:${profileRef}`,
      external_agent_session: sessionRef,
    });
  }

  programNew(nodeId: string, operands?: Json): this {
    return this.op(nodeId, OP_PROGRAM_NEW, operands);
  }

  programInvoke(nodeId: string, operands?: Json): this {
    return this.op(nodeId, OP_PROGRAM_INVOKE, operands);
  }

  awaitEvent(nodeId: string, eventRef: string): this {
    if (!eventRef.trim()) {
      throw new Error("event_ref must not be empty");
    }
    return this.op(nodeId, OP_AWAIT_EVENT, { event_ref: eventRef });
  }

  yieldRegion(regionId: string): this {
    return this.region(regionId, "yield");
  }

  returnRegion(regionId: string): this {
    return this.region(regionId, "return");
  }

  region(regionId: string, kind: string): this {
    (this.graph.structural_regions as Json[]).push({ region_id: regionId, kind });
    return this;
  }

  contextEdge(fromNode: string, toNode: string, contextTypeRef: string): this {
    (this.graph.context_flow as Json[]).push({
      from_node: fromNode,
      to_node: toNode,
      context_type_ref: contextTypeRef,
    });
    return this;
  }

  hook(fields: Json): this {
    (this.graph.hook_bindings as Json[]).push({ ...fields });
    return this;
  }

  capabilityRequirement(capabilityRef: string): this {
    (this.graph.capability_requirements as Json[]).push({ capability_ref: capabilityRef });
    return this;
  }

  modelRequirement(modelTargetRef: string): this {
    (this.graph.model_requirements as Json[]).push({ model_target_ref: modelTargetRef });
    return this;
  }

  annotateRegion(regionId: string, annotation: string): this {
    (this.graph.source_map as Json).region_annotations = [
      ...(((this.graph.source_map as Json).region_annotations as Json[]) ?? []),
      { region_id: regionId, annotation },
    ];
    return this;
  }

  nodeSpan(
    nodeId: string,
    sourceFile: string,
    line: number,
    semanticAnnotation: string,
  ): this {
    (this.graph.source_map as Json).node_spans = [
      ...(((this.graph.source_map as Json).node_spans as Json[]) ?? []),
      {
        node_id: nodeId,
        source_file: sourceFile,
        span: {
          start_line: line,
          start_column: 0,
          end_line: line,
          end_column: 1,
        },
        semantic_annotation: semanticAnnotation,
      },
    ];
    return this;
  }

  build(): Json {
    return JSON.parse(JSON.stringify(this.graph)) as Json;
  }
}

export function verify(graph: Json): string | null {
  return native.verifyFrontendGraph(JSON.stringify(graph));
}

export function lower(graph: Json): Json {
  return JSON.parse(native.lowerFrontendGraph(JSON.stringify(graph))) as Json;
}

export function canonicalAirJson(graph: Json): string {
  return native.lowerFrontendGraph(JSON.stringify(graph));
}

export function compileArtifact(graph: Json): Json {
  return JSON.parse(native.compileFrontendGraphArtifact(JSON.stringify(graph))) as Json;
}
