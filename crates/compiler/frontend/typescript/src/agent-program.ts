// Provides the generic TypeScript AgentProgram authoring surface.

import {
  FrontendGraphRecorder,
  canonicalAirJson,
  lower,
  verify,
  type Json,
} from "./frontend-graph.js";
import {
  type ProgramInstanceRef,
  type ProgramInvokeSpec,
  type ProgramNewSpec,
  type ProgramRef,
  programInvokeOperands,
  programNewOperands,
} from "./program-instance.js";
import {
  STRUCTURAL_AIS_LOOP,
  STRUCTURAL_BRANCH,
  STRUCTURAL_CATCH,
  STRUCTURAL_PARALLEL_JOIN,
  STRUCTURAL_RETURN,
  STRUCTURAL_SWITCH,
  STRUCTURAL_THROW,
  STRUCTURAL_TRY,
  STRUCTURAL_YIELD,
} from "./generated/frontend-contract.js";
import { StructuredTaskScope } from "./structured-task.js";

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
  private readonly recorder: FrontendGraphRecorder;

  constructor(options: {
    program_id: string;
    entrypoint?: string;
    input_type_ref: string;
    output_type_ref: string;
    has_default_context?: boolean;
    context_type_ref?: string;
    source_language?: string;
  }) {
    this.programId = options.program_id;
    this.contextTypeRef = options.context_type_ref;
    this.recorder = new FrontendGraphRecorder(options.source_language ?? "typescript");
    this.recorder.program({
      program_id: options.program_id,
      entrypoint: options.entrypoint ?? "run",
      input_type_ref: options.input_type_ref,
      output_type_ref: options.output_type_ref,
      has_default_context: options.has_default_context ?? true,
      context_type_ref: options.context_type_ref,
    });
  }

  importProgram(imported: ImportedProgram): this {
    this.recorder.importProgram(imported);
    return this;
  }

  bindHook(hook: HookBinding): this {
    this.recorder.hook(hook as Json);
    return this;
  }

  programNew(
    nodeId: string,
    spec?: ProgramNewSpec,
  ): { program: AgentProgram; instance: ProgramInstanceRef } {
    this.recorder.programNew(
      nodeId,
      spec === undefined ? undefined : programNewOperands(spec),
    );
    return {
      program: this,
      instance: {
        instance_node_id: nodeId,
        program_ref: spec?.program_ref ?? nodeId,
      },
    };
  }

  programInvoke(
    nodeId: string,
    receiver: ProgramRef | ProgramInstanceRef,
    input?: Record<string, unknown>,
  ): this {
    this.recorder.programInvoke(nodeId, programInvokeOperands({ receiver, input }));
    return this;
  }

  modelCall(nodeId: string, modelTargetRef?: string): this {
    this.recorder.modelCall(nodeId, modelTargetRef);
    if (modelTargetRef !== undefined) this.recorder.modelRequirement(modelTargetRef);
    return this;
  }

  capabilityInvoke(nodeId: string, capabilityRef?: string): this {
    this.recorder.capabilityInvoke(nodeId, capabilityRef);
    if (capabilityRef !== undefined) {
      this.recorder.capabilityRequirement(capabilityRef);
    }
    return this;
  }

  awaitEvent(nodeId: string, eventRef: string): this {
    this.recorder.awaitEvent(nodeId, eventRef);
    return this;
  }

  loop(regionId: string, body: (program: this) => unknown): this {
    this.recorder.annotateRegion(regionId, "structural_loop");
    return this.structuredRegion(regionId, STRUCTURAL_AIS_LOOP, body);
  }

  branch(
    regionId: string,
    thenBody: (program: this) => unknown,
    elseBody?: (program: this) => unknown,
  ): this {
    return this.structuredRegion(regionId, STRUCTURAL_BRANCH, (program) => {
      thenBody(program);
      elseBody?.(program);
    });
  }

  switch(regionId: string, cases: ReadonlyArray<(program: this) => unknown>): this {
    return this.structuredRegion(regionId, STRUCTURAL_SWITCH, (program) => {
      for (const body of cases) body(program);
    });
  }

  parallel(regionId: string, body: (program: this) => unknown): this {
    return this.structuredRegion(regionId, STRUCTURAL_PARALLEL_JOIN, body);
  }

  tryCatch(
    tryRegionId: string,
    catchRegionId: string,
    tryBody: (program: this) => unknown,
    catchBody: (program: this) => unknown,
  ): this {
    this.structuredRegion(tryRegionId, STRUCTURAL_TRY, tryBody);
    this.structuredRegion(catchRegionId, STRUCTURAL_CATCH, catchBody);
    return this;
  }

  throwRegion(regionId: string): this {
    this.recorder.region(regionId, STRUCTURAL_THROW);
    return this;
  }

  returnRegion(regionId: string): this {
    this.recorder.region(regionId, STRUCTURAL_RETURN);
    return this;
  }

  yieldRegion(regionId: string): this {
    this.recorder.region(regionId, STRUCTURAL_YIELD);
    return this;
  }

  sourceSpan(
    nodeId: string,
    sourceFile: string,
    line: number,
    semanticAnnotation: string,
    startColumn = 0,
    endColumn = startColumn + 1,
  ): this {
    this.recorder.nodeSpan(
      nodeId,
      sourceFile,
      line,
      semanticAnnotation,
      startColumn,
      endColumn,
    );
    return this;
  }

  structuredTask(regionId: string): StructuredTaskScope {
    return new StructuredTaskScope(
      regionId,
      () => {
        this.recorder.region(regionId, STRUCTURAL_PARALLEL_JOIN);
        return this.recorder.enterRegion(regionId);
      },
      (previous) => this.recorder.leaveRegion(previous),
      (nodeId, spec) => this.recorder.programInvoke(nodeId, programInvokeOperands(spec)),
    );
  }

  contextFlow(edge: ContextEdge): this {
    this.recorder.contextEdge(edge.from_node, edge.to_node, edge.context_type_ref);
    return this;
  }

  annotateRegion(regionId: string, annotation: string): this {
    this.recorder.annotateRegion(regionId, annotation);
    return this;
  }

  private structuredRegion(
    regionId: string,
    kind: string,
    body: (program: this) => unknown,
  ): this {
    this.recorder.region(regionId, kind);
    const previous = this.recorder.enterRegion(regionId);
    try {
      body(this);
    } finally {
      this.recorder.leaveRegion(previous);
    }
    return this;
  }

  buildGraph(): Json {
    return this.recorder.build();
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
