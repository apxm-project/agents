import { getHandlerModule, makeHandlerId, TOOL_REGISTRY } from "./registry.js";
import type { HookFnCallable } from "./types.js";

export enum LifecycleEvent {
  SESSION_START = "session_start",
  PRE_TURN = "pre_turn",
  POST_TURN = "post_turn",
  PRE_ASK = "pre_ask",
  POST_ASK = "post_ask",
  PRE_CAP = "pre_cap",
  POST_CAP = "post_cap",
}

export enum HookMode {
  OBSERVE = "observe",
  GATE = "gate",
}

export const LIFECYCLE_EVENTS: ReadonlySet<string> = new Set(
  Object.values(LifecycleEvent),
);

export const GATE_LIFECYCLE_EVENTS: ReadonlySet<string> = new Set([
  LifecycleEvent.SESSION_START,
  LifecycleEvent.PRE_TURN,
  LifecycleEvent.PRE_ASK,
  LifecycleEvent.PRE_CAP,
]);

export const HOOK_MODES: ReadonlySet<string> = new Set(Object.values(HookMode));

export function normalizeLifecycleEvent(value: LifecycleEvent | string): string {
  if (typeof value === "string") {
    return value;
  }
  return value;
}

export function normalizeHookMode(value: HookMode | string): string {
  if (typeof value === "string") {
    return value;
  }
  return value;
}

export interface HookOptions {
  on: LifecycleEvent | string;
  match?: string;
  mode?: HookMode | string;
}

export interface HookFn {
  readonly kind: "hook";
  fn: HookFnCallable;
  event: string;
  match: string;
  mode: string;
  handler_id: string;
  name: string;
  module: string;
  qualname: string;
}

export function isHookFn(value: unknown): value is HookFn {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as HookFn).kind === "hook" &&
    typeof (value as HookFn).handler_id === "string"
  );
}

export function hook(options: HookOptions): (fn: HookFnCallable) => HookFn {
  const eventValue = normalizeLifecycleEvent(options.on);
  const modeValue = normalizeHookMode(options.mode ?? HookMode.OBSERVE);
  const match = options.match ?? "*";

  if (!LIFECYCLE_EVENTS.has(eventValue)) {
    const valid = [...LIFECYCLE_EVENTS].sort().join(", ");
    throw new Error(
      `@hook(on=${JSON.stringify(eventValue)}) is not a valid event; expected one of: ${valid}`,
    );
  }
  if (!HOOK_MODES.has(modeValue)) {
    const valid = [...HOOK_MODES].sort().join(", ");
    throw new Error(
      `@hook(mode=${JSON.stringify(modeValue)}) invalid; expected one of: ${valid}`,
    );
  }
  if (modeValue === HookMode.GATE && !GATE_LIFECYCLE_EVENTS.has(eventValue)) {
    throw new Error(
      `@hook(on=${JSON.stringify(eventValue)}, mode='gate') is invalid; gate mode is only ` +
        `permitted on pre-execution events: ${[...GATE_LIFECYCLE_EVENTS].sort().join(", ")}`,
    );
  }

  return (fn: HookFnCallable) => {
    const module = getHandlerModule();
    const qualname = fn.name || "hook";
    const handler_id = makeHandlerId(module, qualname);

    TOOL_REGISTRY.set(handler_id, fn);

    return {
      kind: "hook",
      fn,
      event: eventValue,
      match,
      mode: modeValue,
      handler_id,
      name: qualname,
      module,
      qualname,
    };
  };
}

/** Build a handler manifest descriptor for a hook. */
export function hookDescriptor(h: HookFn, sourceFile?: string): Record<string, unknown> {
  const descriptor: Record<string, unknown> = {
    handler_id: h.handler_id,
    module: h.module,
    qualname: h.qualname,
    name: h.name,
    event: h.event,
    match: h.match,
    mode: h.mode,
  };
  if (sourceFile) {
    descriptor.source_file = sourceFile;
  }
  return descriptor;
}
