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

export { emitOp, formatAttrValue, quote, isVoidOp } from "./air-emit.js";

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
