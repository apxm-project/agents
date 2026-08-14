// Advanced authoring markers: static Hooks and structured task scopes.

import {
  HOOK_PHASE_AFTER,
  HOOK_PHASE_BEFORE,
  HOOK_SCOPE_NODE,
  type HookPhase,
} from "./generated/frontend-graph.js";

type HookAgent<C = unknown> = {
  context: C;
};

/**
 * A static before/after Hook declaration.
 *
 * It carries no target selector, digest, or return mode of its own: the target
 * is resolved from the authored `target` and the return mode is read off the
 * captured `run` body, so the declaration cannot claim one thing while the body
 * does another. There is no `replace` option for the same reason.
 */
export type HookDecl = {
  readonly phase: HookPhase;
  readonly scope: string;
};

export type HookOptions<C = unknown> = {
  agent: unknown;
  target: unknown;
  scope?: string;
  run(agent: HookAgent<C>): Promise<unknown> | unknown;
};

/**
 * Both Hook phases take the Agent's Context type, so `agent.context` is typed
 * inside `run` without an aliasing cast. The cast was not cosmetic: binding
 * Context to a local is what put a Hook body outside the closed authoring
 * subset, because the local is not a value the capture can resolve.
 */
export const Hook = {
  before<Context = unknown>(options: HookOptions<Context>): HookDecl {
    return { phase: HOOK_PHASE_BEFORE, scope: options.scope ?? HOOK_SCOPE_NODE };
  },
  after<Context = unknown>(options: HookOptions<Context>): HookDecl {
    return { phase: HOOK_PHASE_AFTER, scope: options.scope ?? HOOK_SCOPE_NODE };
  },
};

export const TaskGroup = {
  run(body: (group: unknown) => Promise<void> | void): Promise<void> {
    void body;
    throw new Error("TaskGroup is a compiled structured scope in an Agent body");
  },
};
