/** Hook decision payloads returned by control helpers on {@link HookContext}. */
export type HookDecision = Record<string, unknown>;

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject {
  [key: string]: JsonValue;
}
export type JsonSchema = JsonObject;

/** Runtime context passed to lifecycle hooks. */
export interface HookContext {
  readonly remaining_budget: number | null | undefined;
  /** Structured turn context when the host supplies one (e.g. pre_turn). */
  readonly context: unknown;

  log(...args: unknown[]): void;
  allow(): HookDecision;
  deny(reason?: string): HookDecision;
  editArgs(args: Record<string, unknown>): HookDecision;
  replaceResult(result: unknown): HookDecision;
  prependSystem(text: string): HookDecision;
  setSystem(text: string): HookDecision;
  readAgentsMd(): string;
  recall(key: string): Promise<unknown>;
  recallWindow(n?: number, prefix?: string): Promise<string>;
  umem(key: string, value: unknown): void;
  ask(prompt: string, system?: string | null): Promise<string>;
  call(name: string, args?: Record<string, unknown>): Promise<unknown>;
  countTokens(text: string): Promise<number>;
}

/** Guarded tool/capability call passed to pre/post_cap hooks. */
export interface HookCall {
  readonly name: string;
  readonly args: Record<string, unknown>;
}

export type ToolFn = (...args: unknown[]) => unknown;
export type HookFnCallable = (...args: unknown[]) => unknown;
