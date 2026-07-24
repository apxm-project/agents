// Advanced authoring markers: static Hooks and structured task scopes.

import { stableDigest } from "./markers.js";

type HookAgent<C = unknown> = {
  context: C;
};

export type HookDecl = {
  readonly phase: "before" | "after";
  readonly targetSelector: string;
  readonly scope: string;
  readonly returnMode: "observe" | "replace_result";
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
      phase: "before",
      targetSelector: "static_target",
      scope: options.scope ?? "node",
      returnMode: options.replace ? "replace_result" : "observe",
      handlerDigest: stableDigest("hook:before"),
    };
  },
  after(options: HookOptions): HookDecl {
    return {
      phase: "after",
      targetSelector: "static_target",
      scope: options.scope ?? "node",
      returnMode: options.replace ? "replace_result" : "observe",
      handlerDigest: stableDigest("hook:after"),
    };
  },
};

export const TaskGroup = {
  run(_scope: (group: unknown) => Promise<void> | void): Promise<void> {
    throw new Error("TaskGroup is a compiled structured scope in an Agent body");
  },
};
