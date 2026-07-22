// Conversational Agent Program composition on public five-op authoring APIs.

import {
  AgentProgram,
  type ContextEdge,
  type HookBinding,
  type ImportedProgram,
} from "./agent-program.ts";

export type TurnSpec = {
  model_node_id?: string;
  capability_node_id?: string;
  await_node_id?: string;
  model_target_ref?: string;
  capability_ref?: string;
  event_ref?: string;
};

export type SpecialistComposition = {
  import_ref: ImportedProgram;
  new_node_id?: string;
  invoke_node_id?: string;
};

export class ConversationalAgent extends AgentProgram {
  static readonly LOOP_REGION_SUFFIX = "loop.turn";
  static readonly RETURN_REGION_ID = "region.return";

  readonly loopRegionId: string;
  private specialist: SpecialistComposition | undefined;
  private readonly sourceFile: string;

  constructor(opts: {
    program_id: string;
    input_type_ref: string;
    output_type_ref: string;
    context_type_ref: string;
    entrypoint?: string;
    source_language?: string;
  }) {
    super({
      program_id: opts.program_id,
      entrypoint: opts.entrypoint,
      input_type_ref: opts.input_type_ref,
      output_type_ref: opts.output_type_ref,
      has_default_context: true,
      context_type_ref: opts.context_type_ref,
      source_language: opts.source_language,
    });
    this.loopRegionId = `region.${ConversationalAgent.LOOP_REGION_SUFFIX}`;
    this.sourceFile = `${opts.program_id}.${opts.source_language === "typescript" ? "ts" : "py"}`;
  }

  withSpecialist(composition: SpecialistComposition): this {
    this.specialist = composition;
    return this;
  }

  defineTurn(
    turn: TurnSpec = {},
    opts: { hooks?: HookBinding[] } = {},
  ): this {
    const modelNodeId = turn.model_node_id ?? "node.turn.model";
    const capabilityNodeId = turn.capability_node_id ?? "node.turn.tool";
    const awaitNodeId = turn.await_node_id ?? "node.turn.await";
    const modelTargetRef = turn.model_target_ref ?? "model.default";
    const eventRef = turn.event_ref ?? "event.turn.input";

    if (this.specialist !== undefined) {
      this.importProgram(this.specialist.import_ref);
    }
    this.builder.modelCall(modelNodeId, modelTargetRef);
    if (turn.capability_ref !== undefined) {
      this.builder.capabilityInvoke(capabilityNodeId, turn.capability_ref);
    }
    if (this.specialist !== undefined) {
      this.builder.programNew(this.specialist.new_node_id ?? "node.specialist.new");
      this.builder.programInvoke(this.specialist.invoke_node_id ?? "node.specialist.invoke");
    }
    this.builder.awaitEvent(awaitNodeId, eventRef);
    this.builder.region(this.loopRegionId, "loop");
    this.builder.returnRegion(ConversationalAgent.RETURN_REGION_ID);

    const contextSink =
      turn.capability_ref !== undefined ? capabilityNodeId : awaitNodeId;
    this.contextFlow({
      from_node: modelNodeId,
      to_node: contextSink,
      context_type_ref: this.contextTypeRef ?? "",
    });

    for (const hook of opts.hooks ?? []) {
      this.bindHook(hook);
    }

    this.builder.modelRequirement(modelTargetRef);
    if (turn.capability_ref !== undefined) {
      this.builder.capabilityRequirement(turn.capability_ref);
    }
    const sourceNodes: Array<[string, string]> = [[modelNodeId, "model.call"]];
    if (turn.capability_ref !== undefined) {
      sourceNodes.push([capabilityNodeId, "capability.invoke"]);
    }
    if (this.specialist !== undefined) {
      sourceNodes.push([
        this.specialist.new_node_id ?? "node.specialist.new",
        "program.new",
      ]);
      sourceNodes.push([
        this.specialist.invoke_node_id ?? "node.specialist.invoke",
        "program.invoke",
      ]);
    }
    sourceNodes.push([awaitNodeId, "await.event"]);
    sourceNodes.forEach(([nodeId, annotation], index) => {
      this.builder.nodeSpan(nodeId, this.sourceFile, index + 1, annotation);
    });

    return this.annotateRegion(this.loopRegionId, "conversational_loop");
  }
}
