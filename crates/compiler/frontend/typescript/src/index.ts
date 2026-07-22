// Publishes the generic TypeScript Agent Program authoring and compiler bridge APIs.

export {
  FIVE_OPS,
  FRONTEND_GRAPH_VERSION,
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
} from "./frontend-graph.js";
export {
  AgentProgram,
  type ContextEdge,
  type HookBinding,
  type ImportedProgram,
} from "./agent-program.js";
export { Hook, type AgentFacade, type HookHandler } from "./hook.js";
export {
  type ProgramInstanceRef,
  type ProgramInvokeSpec,
  type ProgramNewSpec,
  type ProgramRef,
  programInstanceReceiver,
  programInvokeOperands,
  programNewOperands,
} from "./program-instance.js";
export { StructuredTaskScope } from "./structured-task.js";
export {
  ALL_FACT_KINDS,
  RUNTIME_FACT_KINDS,
  decodeFact,
  type Fact,
  type LoopIterationCompletedFact,
  type NodeExecutionRecordedFact,
  type NodeExecutionScope,
  type RuntimeFact,
  type RuntimeFactKind,
} from "./generated/runtime-evidence.js";
