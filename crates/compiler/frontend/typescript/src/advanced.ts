// Advanced authoring markers: static Hooks and structured task scopes.

import {
  HOOK_PHASE_AFTER,
  HOOK_PHASE_BEFORE,
  HOOK_RETURN_MODE_OBSERVE,
  HOOK_RETURN_MODE_REPLACE_RESULT,
  HOOK_SCOPE_NODE,
  type HookPhase,
  type HookReturnMode,
} from "./generated/frontend-graph.js";
import { stableDigest } from "./markers.js";

type HookAgent<C = unknown> = {
  context: C;
};

export type HookDecl = {
  readonly phase: HookPhase;
  readonly targetSelector: string;
  readonly scope: string;
  readonly returnMode: HookReturnMode;
  readonly handlerDigest: string;
};

export type HookOptions<C = unknown> = {
  agent: unknown;
  target: unknown;
  scope?: string;
  replace?: boolean;
  run(agent: HookAgent<C>): Promise<unknown> | unknown;
};

export const Hook = {
  before(options: HookOptions): HookDecl {
    return {
      phase: HOOK_PHASE_BEFORE,
      targetSelector: "static_target",
      scope: options.scope ?? HOOK_SCOPE_NODE,
      returnMode: options.replace
        ? HOOK_RETURN_MODE_REPLACE_RESULT
        : HOOK_RETURN_MODE_OBSERVE,
      handlerDigest: stableDigest("hook:before"),
    };
  },
  after(options: HookOptions): HookDecl {
    return {
      phase: HOOK_PHASE_AFTER,
      targetSelector: "static_target",
      scope: options.scope ?? HOOK_SCOPE_NODE,
      returnMode: options.replace
        ? HOOK_RETURN_MODE_REPLACE_RESULT
        : HOOK_RETURN_MODE_OBSERVE,
      handlerDigest: stableDigest("hook:after"),
    };
  },
};

export const TaskGroup = {
  run(_scope: (group: unknown) => Promise<void> | void): Promise<void> {
    throw new Error("TaskGroup is a compiled structured scope in an Agent body");
  },
};
