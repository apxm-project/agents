// Types for package-local TypeScript Capability handlers.

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject {
  [key: string]: JsonValue;
}
export type JsonSchema = JsonObject;

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
  fn: (args: TArgs) => TResult | Promise<TResult>;
}

export interface ToolBuilder {
  <TArgs, TResult>(
    fn: (args: TArgs) => TResult | Promise<TResult>,
  ): FunctionTool<TArgs, TResult>;
}

export function tool<TArgs, TResult>(
  fn: (args: TArgs) => TResult | Promise<TResult>,
): FunctionTool<TArgs, TResult>;
export function tool(options?: ToolOptions): ToolBuilder;
export function isFunctionTool(value: unknown): value is FunctionTool;
export function makeHandlerId(module: string, qualname: string): string;
