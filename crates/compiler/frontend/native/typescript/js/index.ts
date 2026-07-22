// Canonical Agent Program authoring frontend (TypeScript).
//
// Author a program with the five semantic operations and structural regions,
// then lower the recorded FrontendGraph to canonical AIR through the in-process
// Node-API addon. No CLI subprocess and no network compile are involved.

export {
  FIVE_OPS,
  FRONTEND_GRAPH_VERSION,
  GraphBuilder,
  OP_AWAIT_EVENT,
  OP_CAPABILITY_INVOKE,
  OP_MODEL_CALL,
  OP_PROGRAM_INVOKE,
  OP_PROGRAM_NEW,
  SOURCE_MAP_VERSION,
  canonicalAirJson,
  compileArtifact,
  lower,
  verify,
  type Json,
  type OpName,
} from "./graph-builder.ts";

export { AgentProgram, type ContextEdge, type HookBinding, type ImportedProgram } from "./agent-program.ts";
export {
  ConversationalAgent,
  type SpecialistComposition,
  type TurnSpec,
} from "./conversational.ts";
export { Gao, gaoConversationalGraph } from "./gao.ts";
export { Hook } from "./hook.ts";
export type { AgentFacade, HookHandler } from "./hook.ts";
export {
  type ProgramInstanceRef,
  type ProgramInvokeSpec,
  type ProgramNewSpec,
  type ProgramRef,
  programInstanceReceiver,
  programInvokeOperands,
  programNewOperands,
} from "./program-instance.ts";
export { StructuredTaskScope } from "./structured-task.ts";
