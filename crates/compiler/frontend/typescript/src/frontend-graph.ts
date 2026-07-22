// Records canonical FrontendGraph values and invokes the explicit native bridge.

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  INTERNAL_STRUCTURAL_REGION,
  SEMANTIC_AWAIT_EVENT as OP_AWAIT_EVENT,
  SEMANTIC_CAPABILITY_INVOKE as OP_CAPABILITY_INVOKE,
  SEMANTIC_MODEL_CALL as OP_MODEL_CALL,
  SEMANTIC_OPERATION_KINDS,
  SEMANTIC_PROGRAM_INVOKE as OP_PROGRAM_INVOKE,
  SEMANTIC_PROGRAM_NEW as OP_PROGRAM_NEW,
} from "./generated/frontend-contract.js";

const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));

type NativeBridge = {
  compileFrontendGraphArtifact(json: string): string;
  lowerFrontendGraph(json: string): string;
  verifyFrontendGraph(json: string): string | null;
};

let loadedBridge: NativeBridge | undefined;

function nativeBridge(): NativeBridge {
  if (loadedBridge === undefined) {
    try {
      loadedBridge = require(join(here, "_native.node")) as NativeBridge;
    } catch (cause) {
      throw new Error(
        "The @apxm/frontend native compiler bridge is not installed for this platform",
        { cause },
      );
    }
  }
  return loadedBridge;
}

export {
  OP_AWAIT_EVENT,
  OP_CAPABILITY_INVOKE,
  OP_MODEL_CALL,
  OP_PROGRAM_INVOKE,
  OP_PROGRAM_NEW,
};

export type OpName = (typeof SEMANTIC_OPERATION_KINDS)[number];

export const FIVE_OPS: ReadonlySet<OpName> = new Set(SEMANTIC_OPERATION_KINDS);

export const FRONTEND_GRAPH_VERSION = "apxm.frontend-graph.v1";
export const SOURCE_MAP_VERSION = "apxm.source-map.v1";

export type Json = Record<string, unknown>;

export class FrontendGraphRecorder {
  private readonly graph: Json;
  private parentRegionId: string | undefined;
  private readonly nextExecutionOrder = new Map<string | undefined, number>();

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

  program(definition: {
    program_id: string;
    entrypoint: string;
    input_type_ref: string;
    output_type_ref: string;
    has_default_context: boolean;
    context_type_ref?: string;
  }): this {
    const record: Json = {
      program_id: definition.program_id,
      entrypoint: definition.entrypoint,
      input_type_ref: definition.input_type_ref,
      output_type_ref: definition.output_type_ref,
      has_default_context: definition.has_default_context,
    };
    if (definition.context_type_ref !== undefined) {
      record.context_type_ref = definition.context_type_ref;
    }
    (this.graph.program_definitions as Json[]).push(record);
    const rootRegionId = `region.${definition.program_id}.body`;
    this.region(rootRegionId, INTERNAL_STRUCTURAL_REGION);
    this.parentRegionId = rootRegionId;
    return this;
  }

  importProgram(reference: {
    program_ref: string;
    artifact_digest: string;
    entrypoint: string;
    target_agent_identity_requirement: string;
  }): this {
    (this.graph.imported_program_refs as Json[]).push({ ...reference });
    return this;
  }

  private operation(nodeId: string, operation: OpName, operands?: Json): this {
    if (this.parentRegionId === undefined) {
      throw new Error("semantic operations require a program body region");
    }
    const record: Json = {
      node_id: nodeId,
      op: operation,
      parent_region_id: this.parentRegionId,
      execution_order: this.takeExecutionOrder(this.parentRegionId),
    };
    if (operands !== undefined) record.operands = operands;
    (this.graph.semantic_operations as Json[]).push(record);
    return this;
  }

  modelCall(nodeId: string, modelTargetRef?: string): this {
    return this.operation(
      nodeId,
      OP_MODEL_CALL,
      modelTargetRef === undefined ? undefined : { model_target_ref: modelTargetRef },
    );
  }

  capabilityInvoke(nodeId: string, capabilityRef?: string): this {
    return this.operation(
      nodeId,
      OP_CAPABILITY_INVOKE,
      capabilityRef === undefined ? undefined : { capability_ref: capabilityRef },
    );
  }

  externalAgentCapability(nodeId: string, profileRef: string, sessionRef: string): this {
    return this.operation(nodeId, OP_CAPABILITY_INVOKE, {
      capability_ref: `external-agent:${profileRef}`,
      external_agent_session: sessionRef,
    });
  }

  programNew(nodeId: string, operands?: Json): this {
    return this.operation(nodeId, OP_PROGRAM_NEW, operands);
  }

  programInvoke(nodeId: string, operands?: Json): this {
    return this.operation(nodeId, OP_PROGRAM_INVOKE, operands);
  }

  awaitEvent(nodeId: string, eventRef: string): this {
    if (!eventRef.trim()) throw new Error("event_ref must not be empty");
    return this.operation(nodeId, OP_AWAIT_EVENT, { event_ref: eventRef });
  }

  region(regionId: string, kind: string): this {
    const record: Json = {
      region_id: regionId,
      kind,
      execution_order: this.takeExecutionOrder(this.parentRegionId),
    };
    if (this.parentRegionId !== undefined) {
      record.parent_region_id = this.parentRegionId;
    }
    (this.graph.structural_regions as Json[]).push(record);
    return this;
  }

  enterRegion(regionId: string): string | undefined {
    const previous = this.parentRegionId;
    this.parentRegionId = regionId;
    return previous;
  }

  leaveRegion(previous: string | undefined): void {
    this.parentRegionId = previous;
  }

  private takeExecutionOrder(parentRegionId: string | undefined): number {
    const order = this.nextExecutionOrder.get(parentRegionId) ?? 0;
    this.nextExecutionOrder.set(parentRegionId, order + 1);
    return order;
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
    const sourceMap = this.graph.source_map as Json;
    (sourceMap.region_annotations as Json[]).push({ region_id: regionId, annotation });
    return this;
  }

  nodeSpan(
    nodeId: string,
    sourceFile: string,
    line: number,
    semanticAnnotation: string,
    startColumn = 0,
    endColumn = startColumn + 1,
  ): this {
    const sourceMap = this.graph.source_map as Json;
    (sourceMap.node_spans as Json[]).push({
      node_id: nodeId,
      source_file: sourceFile,
      span: {
        start_line: line,
        start_column: startColumn,
        end_line: line,
        end_column: endColumn,
      },
      semantic_annotation: semanticAnnotation,
    });
    return this;
  }

  build(): Json {
    return structuredClone(this.graph) as Json;
  }
}

/** Verify a recorded FrontendGraph through the native compiler bridge. */
export function verify(graph: Json): string | null {
  return nativeBridge().verifyFrontendGraph(JSON.stringify(graph));
}

/** Lower a recorded FrontendGraph through the native compiler bridge. */
export function lower(graph: Json): Json {
  return JSON.parse(nativeBridge().lowerFrontendGraph(JSON.stringify(graph))) as Json;
}

/** Return canonical AIR JSON from the native compiler bridge. */
export function canonicalAirJson(graph: Json): string {
  return nativeBridge().lowerFrontendGraph(JSON.stringify(graph));
}

/** Compile a recorded FrontendGraph into an executable artifact. */
export function compileArtifact(graph: Json): Json {
  return JSON.parse(
    nativeBridge().compileFrontendGraphArtifact(JSON.stringify(graph)),
  ) as Json;
}
