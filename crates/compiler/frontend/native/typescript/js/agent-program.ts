// Typed Agent Program authoring on the canonical five-op FrontendGraph surface.

import {
  GraphBuilder,
  canonicalAirJson,
  lower,
  verify,
  type Json,
} from "./graph-builder.ts";
import {
  type ProgramInstanceRef,
  type ProgramInvokeSpec,
  type ProgramNewSpec,
  type ProgramRef,
  programInvokeOperands,
  programNewOperands,
} from "./program-instance.ts";
import { StructuredTaskScope } from "./structured-task.ts";

export type HookBinding = {
  hook_id: string;
  scope: "agent" | "loop" | "node" | "model" | "capability";
  phase: "before" | "after";
  target_selector: string;
  declaration_order: number;
  handler_ref: string;
  handler_digest: string;
  input_type_ref: string;
  output_type_ref: string;
  return_mode: "observe" | "replace_result";
};

export type ContextEdge = {
  from_node: string;
  to_node: string;
  context_type_ref: string;
};

export type ImportedProgram = {
  program_ref: string;
  artifact_digest: string;
  entrypoint: string;
  target_agent_identity_requirement: string;
};

export class AgentProgram {
  readonly programId: string;
  readonly contextTypeRef: string | undefined;
  protected builder: GraphBuilder;

  constructor(opts: {
    program_id: string;
    entrypoint?: string;
    input_type_ref: string;
    output_type_ref: string;
    has_default_context?: boolean;
    context_type_ref?: string;
    source_language?: string;
  }) {
    this.programId = opts.program_id;
    this.contextTypeRef = opts.context_type_ref;
    this.builder = new GraphBuilder(opts.source_language ?? "typescript");
    this.builder.program({
      program_id: opts.program_id,
      entrypoint: opts.entrypoint ?? "run",
      input_type_ref: opts.input_type_ref,
      output_type_ref: opts.output_type_ref,
      has_default_context: opts.has_default_context ?? true,
      context_type_ref: opts.context_type_ref,
    });
  }

  importProgram(imported: ImportedProgram): this {
    this.builder.importProgram(imported);
    return this;
  }

  bindHook(hook: HookBinding): this {
    this.builder.hook(hook as Json);
    return this;
  }

  programNew(
    nodeId: string,
    spec?: ProgramNewSpec,
  ): { program: this; instance: ProgramInstanceRef } {
    this.builder.programNew(nodeId, spec === undefined ? undefined : programNewOperands(spec));
    const programRef = spec?.program_ref ?? nodeId;
    return { program: this, instance: { instance_node_id: nodeId, program_ref: programRef } };
  }

  programInvoke(
    nodeId: string,
    receiver: ProgramRef | ProgramInstanceRef,
    input?: Record<string, unknown>,
  ): this {
    const spec: ProgramInvokeSpec = { receiver, input };
    this.builder.programInvoke(nodeId, programInvokeOperands(spec));
    return this;
  }

  structuredTask(regionId: string): StructuredTaskScope {
    return new StructuredTaskScope(this, regionId);
  }

  contextFlow(edge: ContextEdge): this {
    this.builder.contextEdge(edge.from_node, edge.to_node, edge.context_type_ref);
    return this;
  }

  annotateRegion(regionId: string, annotation: string): this {
    this.builder.annotateRegion(regionId, annotation);
    return this;
  }

  buildGraph(): Json {
    return this.builder.build();
  }

  verifyGraph(): string | null {
    return verify(this.buildGraph());
  }

  lowerGraph(): Json {
    return lower(this.buildGraph());
  }

  canonicalAir(): string {
    return canonicalAirJson(this.buildGraph());
  }
}
