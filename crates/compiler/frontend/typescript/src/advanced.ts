// Advanced authoring markers: static Hooks and structured task scopes.

import type { AgentHandle } from "./agent.js";
import {
  HOOK_PHASE_AFTER,
  HOOK_PHASE_BEFORE,
  HOOK_SCOPE_NODE,
  type HookPhase,
} from "./generated/frontend-graph.js";
import type { Scope } from "./generated/scopes.js";

type HookAgent<C = unknown> = {
  context: C;
};

/**
 * Anything a Hook can wrap: one declaration the module made, or the Agent
 * itself. The kind is what the capture resolves the target through, so the
 * type states the same closed set the capture does.
 */
export type HookTarget = {
  readonly kind:
    | "model_binding"
    | "tool_binding"
    | "capability_binding"
    | "event_type"
    | "skill"
    | "agent_definition";
};

/**
 * A static before/after Hook declaration.
 *
 * It carries no target selector, digest, or return mode of its own: the target
 * is resolved from the authored `target` and the return mode is read off the
 * captured `run` body, so the declaration cannot claim one thing while the body
 * does another. There is no `replace` option for the same reason.
 */
export type HookDecl<C = unknown> = {
  readonly phase: HookPhase;
  readonly scope: Scope;
  readonly agent: AgentHandle<never, unknown, C>;
  readonly target: HookTarget;
  readonly run: (agent: HookAgent<C>) => Promise<unknown> | unknown;
};

export type HookOptions<C = unknown> = {
  agent: AgentHandle<never, unknown, C>;
  target: HookTarget;
  scope?: Scope;
  run(agent: HookAgent<C>): Promise<unknown> | unknown;
};

/**
 * Both Hook phases take the Agent's Context type, so `agent.context` is typed
 * inside `run` without an aliasing cast. The cast was not cosmetic: binding
 * Context to a local is what put a Hook body outside the closed authoring
 * subset, because the local is not a value the capture can resolve.
 */
export const Hook = {
  before<Context = unknown>(options: HookOptions<Context>): HookDecl<Context> {
    return declare(HOOK_PHASE_BEFORE, options);
  },
  after<Context = unknown>(options: HookOptions<Context>): HookDecl<Context> {
    return declare(HOOK_PHASE_AFTER, options);
  },
};

/** Hold one Hook's authored arguments, so nothing it was given is dropped. */
function declare<C>(phase: HookPhase, options: HookOptions<C>): HookDecl<C> {
  return {
    phase,
    scope: options.scope ?? HOOK_SCOPE_NODE,
    agent: options.agent,
    target: options.target,
    run: options.run,
  };
}

export const TaskGroup = {
  run(body: (group: unknown) => Promise<void> | void): Promise<void> {
    void body;
    throw new Error("TaskGroup is a compiled structured scope in an Agent body");
  },
};
