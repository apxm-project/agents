import { getHandlerModule, makeHandlerId, TOOL_REGISTRY } from "./registry.js";
import type { HandlerFn, JsonSchema, ToolFn } from "./types.js";

export interface ToolOptions {
  name?: string;
  description?: string;
  schema?: JsonSchema;
}

export interface FunctionTool<TArgs = never, TResult = unknown> {
  readonly kind: "tool";
  name: string;
  description: string;
  schema: JsonSchema;
  handler_id: string;
  module: string;
  qualname: string;
  fn: ToolFn<TArgs, TResult>;
}

export function isFunctionTool(value: unknown): value is FunctionTool {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as FunctionTool).kind === "tool" &&
    typeof (value as FunctionTool).handler_id === "string"
  );
}

function wrapTool<TArgs, TResult>(
  fn: ToolFn<TArgs, TResult>,
  options: ToolOptions = {},
): FunctionTool<TArgs, TResult> {
  const module = getHandlerModule();
  const qualname = fn.name || "tool";
  const handler_id = makeHandlerId(module, qualname);

  const ft: FunctionTool<TArgs, TResult> = {
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

export interface ToolBuilder {
  <TArgs, TResult>(fn: ToolFn<TArgs, TResult>): FunctionTool<TArgs, TResult>;
}

/** Decorate or build a TypeScript handler function as an APXM tool. */
export function tool<TArgs, TResult>(
  fn: ToolFn<TArgs, TResult>,
): FunctionTool<TArgs, TResult>;
export function tool(options?: ToolOptions): ToolBuilder;
export function tool(
  fnOrOptions?: HandlerFn | ToolOptions,
): FunctionTool | ToolBuilder {
  if (typeof fnOrOptions === "function") {
    return wrapTool(fnOrOptions);
  }
  const options = fnOrOptions ?? {};
  return <TArgs, TResult>(fn: ToolFn<TArgs, TResult>) => wrapTool(fn, options);
}
