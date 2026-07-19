// The shared authoring example used to prove Python/TypeScript parity.
// The Python frontend authors the identical program, so both lower to
// byte-identical canonical AIR.

import { GraphBuilder } from "./index.ts";

const DIGEST_SUMMARIZER = "sha256:" + "a".repeat(64);
const DIGEST_HOOK = "sha256:" + "b".repeat(64);

export function specialistGraph(): Record<string, unknown> {
  const builder = new GraphBuilder("python");
  builder.program({
    program_id: "Specialist",
    entrypoint: "run",
    input_type_ref: "SpecialistInput",
    output_type_ref: "SpecialistOutput",
    has_default_context: true,
    context_type_ref: "SpecialistContext",
  });
  builder.importProgram({
    program_ref: "Summarizer",
    artifact_digest: DIGEST_SUMMARIZER,
    entrypoint: "run",
    target_agent_identity_requirement: "summarizer-identity",
  });
  builder.modelCall("node.model.1", "model.default");
  builder.capabilityInvoke("node.cap.1", "cap.search");
  builder.programNew("node.new.1");
  builder.programInvoke("node.invoke.1");
  builder.awaitEvent("node.await.1");
  builder.region("region.loop.1", "loop");
  builder.region("region.return.1", "return");
  builder.contextEdge("node.model.1", "node.cap.1", "SpecialistContext");
  builder.hook({
    hook_id: "hook.before.model",
    scope: "model",
    phase: "before",
    target_selector: "node.model.1",
    declaration_order: 0,
    handler_ref: "hooks.before_model",
    handler_digest: DIGEST_HOOK,
    input_type_ref: "ModelContext",
    output_type_ref: "ModelContext",
    return_mode: "observe",
  });
  builder.capabilityRequirement("cap.search");
  builder.modelRequirement("model.default");
  return builder.build();
}

export function gaoConversationalGraph(): Record<string, unknown> {
  // A Gao-style Conversational Agent authored on public constructs: a structural
  // loop region for the conversational loop, a Hook-guarded model.call Turn with
  // explicit context flow, a specialist composed through program.new/
  // program.invoke, and an await.event. Lowers to only the five semantic ops.
  const builder = new GraphBuilder("python");
  builder.program({
    program_id: "Gao",
    entrypoint: "run",
    input_type_ref: "GaoInput",
    output_type_ref: "GaoOutput",
    has_default_context: true,
    context_type_ref: "GaoContext",
  });
  builder.importProgram({
    program_ref: "Specialist",
    artifact_digest: "sha256:" + "c".repeat(64),
    entrypoint: "run",
    target_agent_identity_requirement: "specialist-identity",
  });
  builder.modelCall("node.turn.model", "model.default");
  builder.capabilityInvoke("node.turn.tool", "cap.search");
  builder.programNew("node.specialist.new");
  builder.programInvoke("node.specialist.invoke");
  builder.awaitEvent("node.turn.await");
  builder.region("region.loop.turn", "loop");
  builder.region("region.return", "return");
  builder.contextEdge("node.turn.model", "node.turn.tool", "GaoContext");
  builder.hook({
    hook_id: "hook.before.turn",
    scope: "loop",
    phase: "before",
    target_selector: "region.loop.turn",
    declaration_order: 0,
    handler_ref: "hooks.before_turn",
    handler_digest: "sha256:" + "d".repeat(64),
    input_type_ref: "GaoContext",
    output_type_ref: "GaoContext",
    return_mode: "observe",
  });
  builder.hook({
    hook_id: "hook.after.model",
    scope: "model",
    phase: "after",
    target_selector: "node.turn.model",
    declaration_order: 1,
    handler_ref: "hooks.after_model",
    handler_digest: "sha256:" + "e".repeat(64),
    input_type_ref: "ModelResult",
    output_type_ref: "ModelResult",
    return_mode: "replace_result",
  });
  builder.capabilityRequirement("cap.search");
  builder.modelRequirement("model.default");
  const graph = builder.build() as Record<string, unknown>;
  const sourceMap = graph.source_map as { region_annotations: unknown[] };
  sourceMap.region_annotations.push({ region_id: "region.loop.turn", annotation: "conversational_loop" });
  return graph;
}

export function externalAgentGraph(): Record<string, unknown> {
  const builder = new GraphBuilder("python");
  builder.program({
    program_id: "Delegator",
    entrypoint: "run",
    input_type_ref: "DelegatorInput",
    output_type_ref: "DelegatorOutput",
    has_default_context: true,
    context_type_ref: "DelegatorContext",
  });
  builder.externalAgentCapability("node.acp.1", "acp:claude-code", "session.acp.1");
  builder.region("region.return.1", "return");
  builder.capabilityRequirement("external-agent:acp:claude-code");
  return builder.build();
}
