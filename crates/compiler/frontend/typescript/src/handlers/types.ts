/** Hook decision payloads returned by control helpers on {@link HookContext}. */
export type HookDecision =
  | { decision: "allow" }
  | { decision: "deny"; reason: string }
  | { decision: "edit_args"; args: Record<string, unknown> }
  | { decision: "replace_result"; result: unknown }
  | { decision: "prepend_system"; text: string }
  | { decision: "set_system"; text: string };

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject {
  [key: string]: JsonValue;
}
export type JsonSchema = JsonObject;

/** Stored callable shape used only for registration and runtime reflection. */
export type HandlerFn = (...args: never[]) => unknown;

/** Runtime context passed to lifecycle hooks. */
export interface HookContext {
  readonly remainingBudget: number | null | undefined;
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

export type ToolFn<TArgs, TResult = unknown> = (
  args: TArgs,
) => TResult | Promise<TResult>;

export type HookResult = HookDecision | null | void | Promise<HookDecision | null | void>;
export type ContextHookFn = (ctx: HookContext) => HookResult;
export type CapabilityHookFn = (ctx: HookContext, call: HookCall) => HookResult;
export type CapabilityResultHookFn = (
  ctx: HookContext,
  call: HookCall,
  result: unknown,
) => HookResult;
export type ReplyHookFn = (ctx: HookContext, reply: unknown) => HookResult;
export type HookFnCallable =
  | ContextHookFn
  | CapabilityHookFn
  | CapabilityResultHookFn
  | ReplyHookFn;
