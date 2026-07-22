// The shared authoring example used to prove Python/TypeScript parity.
// The Python frontend authors the identical program, so both lower to
// byte-identical canonical AIR.

import { GraphBuilder } from "./graph-builder.ts";
import { gaoConversationalGraph as buildGaoGraph } from "./gao.ts";

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
  builder.awaitEvent("node.await.1", "event.specialist.await");
  builder.region("region.loop.1", "loop");
  builder.returnRegion("region.return.1");
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
  return buildGaoGraph();
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
  builder.returnRegion("region.return.1");
  builder.capabilityRequirement("external-agent:acp:claude-code");
  return builder.build();
}
