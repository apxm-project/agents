// Advanced authoring markers: static Hooks and structured task scopes.

export type HookDecl = {
  readonly phase: "before" | "after";
  readonly targetSelector: string;
  readonly scope: string;
  readonly returnMode: "observe" | "replace_result";
};

export type HookOptions = {
  target: string;
  scope?: string;
  replace?: boolean;
};

export const Hook = {
  before(options: HookOptions): HookDecl {
    return {
      phase: "before",
      targetSelector: options.target,
      scope: options.scope ?? "node",
      returnMode: options.replace ? "replace_result" : "observe",
    };
  },
  after(options: HookOptions): HookDecl {
    return {
      phase: "after",
      targetSelector: options.target,
      scope: options.scope ?? "node",
      returnMode: options.replace ? "replace_result" : "observe",
    };
  },
};

export const TaskGroup = {
  run(_scope: (group: unknown) => Promise<void> | void): never {
    throw new Error("TaskGroup is a compiled structured scope in an Agent body");
  },
};
