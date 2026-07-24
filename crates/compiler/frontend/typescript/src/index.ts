// Typed Agent authoring in ordinary TypeScript.
//
// An everyday author needs five names — Agent, Context, Tool, Model, and
// ordinary control flow — plus the inferred agent callback parameter. Advanced
// programs add Capability, Event, Hook, and TaskGroup. The frontend reads the
// authored source statically and traverses it into the language-neutral
// FrontendGraph; it never executes the authored body and exposes no operation
// constant, node or region identity, or raw graph builder.

export {
  Agent,
  AgentDefinition,
  ProgramInstance,
  type AgentConfig,
  type AgentFacade,
} from "./agent.js";
export {
  Capability,
  Context,
  Event,
  Model,
  Tool,
  type CapabilityBinding,
  type ContextSchema,
  type EventTypeBinding,
  type HandlerSpec,
  type ModelBinding,
  type ToolBinding,
} from "./markers.js";
export { Hook, TaskGroup, type HookDecl, type HookOptions } from "./advanced.js";
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
