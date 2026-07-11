import { getHandlerModule, makeHandlerId, TOOL_REGISTRY } from "./registry.js";
import type { JsonSchema, ToolFn } from "./types.js";

export interface ToolOptions {
  name?: string;
  description?: string;
  schema?: JsonSchema;
}

export interface FunctionTool {
  readonly kind: "tool";
  name: string;
  description: string;
  schema: JsonSchema;
  handler_id: string;
  module: string;
  qualname: string;
  fn: ToolFn;
}

export function isFunctionTool(value: unknown): value is FunctionTool {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as FunctionTool).kind === "tool" &&
    typeof (value as FunctionTool).handler_id === "string"
  );
}

function wrapTool(fn: ToolFn, options: ToolOptions = {}): FunctionTool {
  const module = getHandlerModule();
  const qualname = fn.name || "tool";
  const handler_id = makeHandlerId(module, qualname);

  const ft: FunctionTool = {
    kind: "tool",
    name: options.name ?? qualname,
    description: options.description ?? "",
    schema: options.schema ?? {},
    handler_id,
    module,
    qualname,
    fn,
  };

  TOOL_REGISTRY.set(handler_id, fn);
  return ft;
}

/** Decorate or build a TypeScript handler function as an APXM tool. */
export function tool(fn: ToolFn): FunctionTool;
export function tool(options?: ToolOptions): (fn: ToolFn) => FunctionTool;
export function tool(
  fnOrOptions?: ToolFn | ToolOptions,
): FunctionTool | ((fn: ToolFn) => FunctionTool) {
  if (typeof fnOrOptions === "function") {
    return wrapTool(fnOrOptions);
  }
  const options = fnOrOptions ?? {};
  return (fn: ToolFn) => wrapTool(fn, options);
}
