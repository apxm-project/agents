// Canonical Agent Program authoring frontend (TypeScript).
//
// Author a program with the five semantic operations and structural regions,
// then lower the recorded FrontendGraph to canonical AIR through the in-process
// Node-API addon. No CLI subprocess and no network compile are involved.

import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));
const native = require(join(here, "_native.node")) as {
  lowerFrontendGraph(json: string): string;
  verifyFrontendGraph(json: string): string | null;
};

export const FRONTEND_GRAPH_VERSION = "apxm.frontend-graph.v1";
export const SOURCE_MAP_VERSION = "apxm.source-map.v1";

type Json = Record<string, unknown>;

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
    return this.op(nodeId, "model.call", modelTargetRef === undefined ? undefined : { model_target_ref: modelTargetRef });
  }

  capabilityInvoke(nodeId: string, capabilityRef?: string): this {
    return this.op(nodeId, "capability.invoke", capabilityRef === undefined ? undefined : { capability_ref: capabilityRef });
  }

  externalAgentCapability(nodeId: string, profileRef: string, sessionRef: string): this {
    // Lowers to exactly one capability.invoke — no new AIR op and no native
    // model.call. The peer's private model/tool cycles are nested attributed
    // evidence under this one NodeExecution; the opaque session reference is the
    // only handle retained in Context.
    return this.op(nodeId, "capability.invoke", {
      capability_ref: `external-agent:${profileRef}`,
      external_agent_session: sessionRef,
    });
  }

  programNew(nodeId: string): this {
    return this.op(nodeId, "program.new");
  }

  programInvoke(nodeId: string): this {
    return this.op(nodeId, "program.invoke");
  }

  awaitEvent(nodeId: string): this {
    return this.op(nodeId, "await.event");
  }

  region(regionId: string, kind: string): this {
    (this.graph.structural_regions as Json[]).push({ region_id: regionId, kind });
    return this;
  }

  contextEdge(fromNode: string, toNode: string, contextTypeRef: string): this {
    (this.graph.context_flow as Json[]).push({ from_node: fromNode, to_node: toNode, context_type_ref: contextTypeRef });
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
