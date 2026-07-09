/** Hook decision payloads returned by control helpers on {@link HookContext}. */
export type HookDecision = Record<string, unknown>;

/** Runtime context passed to lifecycle hooks. */
export interface HookContext {
  readonly remaining_budget: number | null | undefined;
  /** Structured turn context when the host supplies one (e.g. pre_turn). */
  readonly context: unknown;

  log(...args: unknown[]): void;
  allow(): HookDecision;
  deny(reason?: string): HookDecision;
  edit_args(args: Record<string, unknown>): HookDecision;
  editArgs(args: Record<string, unknown>): HookDecision;
  replace_result(result: unknown): HookDecision;
  replaceResult(result: unknown): HookDecision;
  prepend_system(text: string): HookDecision;
  prependSystem(text: string): HookDecision;
  set_system(text: string): HookDecision;
  setSystem(text: string): HookDecision;
  read_agents_md(): string;
  readAgentsMd(): string;
  recall(key: string): unknown;
  recall_window(n?: number, prefix?: string): string;
  recallWindow(n?: number, prefix?: string): string;
  umem(key: string, value: unknown): void;
  ask(prompt: string, system?: string | null): string;
  call(name: string, args?: Record<string, unknown>): unknown;
  count_tokens(text: string): number;
  countTokens(text: string): number;
}

/** Guarded tool/capability call passed to pre/post_cap hooks. */
export interface HookCall {
  readonly name: string;
  readonly args: Record<string, unknown>;
}

export type ToolFn = (...args: unknown[]) => unknown;
export type HookFnCallable = (...args: unknown[]) => unknown;
