export { GraphBuilder, NodeRef } from "./builder.js";
export type {
  AskOptions,
  CommunicateOptions,
  DelegateOptions,
  SpawnAgentOptions,
  InvokeCapabilityOptions,
} from "./builder.js";

export { ApxmGraph, makeEdge, sanitizeFlowName, emitMultiFlowModule } from "./graph.js";
export type { GraphNode, GraphEdge, Parameter, ApxmGraphData } from "./graph.js";

export { validateWorkflowDraft } from "./validate.js";
export type { WorkflowDraftValidationResult } from "./validate.js";

// Vendored workflow-draft.v1 schema ( codegen). Exposed so consumers
// (e.g. Studio's Gao package) can derive human/LLM-readable material — such
// as an authoring rules section — from the same schema `validateWorkflowDraft`
// checks against, instead of hand-maintaining a parallel description.
export { WORKFLOW_DRAFT_V1_SCHEMA } from "./generated/workflow-draft-schema.js";

export {
  OP_SPECS,
  ALL_OPERATIONS,
  VOID_OPS,
  REQUIRED_ATTRS,
  OP_SPEC_SCHEMA_VERSION,
  OP_SPEC_TOTAL_OPERATIONS,
} from "./generated/ops.js";
export type { OpName, OpSpec, OpField } from "./generated/ops.js";

export type { DependencyType, ParamType } from "./types.js";
export { normalizeDependencyType, VALID_PARAM_TYPES } from "./types.js";

export {
  TOOL_REGISTRY,
  getHandlerModule,
  makeHandlerId,
  tool,
  isFunctionTool,
  hook,
  hookDescriptor,
  isHookFn,
  normalizeHookMode,
  normalizeLifecycleEvent,
  LifecycleEvent,
  HookMode,
  LIFECYCLE_EVENTS,
  GATE_LIFECYCLE_EVENTS,
  HOOK_MODES,
} from "./handlers/index.js";
export type {
  HookCall,
  HookContext,
  HookDecision,
  HookFnCallable,
  ToolFn,
  FunctionTool,
  ToolOptions,
  HookFn,
  HookOptions,
} from "./handlers/index.js";

export { compileHandlers } from "./compile-handlers.js";
export type { HandlerManifestEntry } from "./compile-handlers.js";
