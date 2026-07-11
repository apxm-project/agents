export type {
  CapabilityHookFn,
  CapabilityResultHookFn,
  ContextHookFn,
  HandlerFn,
  HookCall,
  HookContext,
  HookDecision,
  HookFnCallable,
  HookResult,
  JsonObject,
  JsonPrimitive,
  JsonSchema,
  JsonValue,
  ReplyHookFn,
  ToolFn,
} from "./types.js";

export { TOOL_REGISTRY, getHandlerModule, makeHandlerId } from "./registry.js";

export { tool, isFunctionTool } from "./tool.js";
export type { FunctionTool, ToolBuilder, ToolOptions } from "./tool.js";

export {
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
} from "./hook.js";
export type { HookBuilder, HookFn, HookOptions } from "./hook.js";
