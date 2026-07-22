// Defines generic static Hook bindings and portable Agent Facade types.

import type { HookBinding } from "./agent-program.js";

export type AgentFacade<C> = {
  context: C;
};

export type HookHandler<C, T> = (agent: AgentFacade<C>) => T | Promise<T>;

export class Hook {
  constructor(private readonly binding: HookBinding) {}

  /** Return the static FrontendGraph Hook binding. */
  toBinding(): HookBinding {
    return { ...this.binding };
  }

  /** Bind an observer before a loop region. */
  static beforeLoop(options: {
    hook_id: string;
    target_selector: string;
    handler_ref: string;
    handler_digest: string;
    context_type_ref: string;
    declaration_order?: number;
  }): Hook {
    return new Hook({
      hook_id: options.hook_id,
      scope: "loop",
      phase: "before",
      target_selector: options.target_selector,
      declaration_order: options.declaration_order ?? 0,
      handler_ref: options.handler_ref,
      handler_digest: options.handler_digest,
      input_type_ref: options.context_type_ref,
      output_type_ref: options.context_type_ref,
      return_mode: "observe",
    });
  }

  /** Bind a result-transforming callback after a model call. */
  static afterModel(options: {
    hook_id: string;
    target_selector: string;
    handler_ref: string;
    handler_digest: string;
    result_type_ref: string;
    declaration_order?: number;
    return_mode?: HookBinding["return_mode"];
  }): Hook {
    return new Hook({
      hook_id: options.hook_id,
      scope: "model",
      phase: "after",
      target_selector: options.target_selector,
      declaration_order: options.declaration_order ?? 0,
      handler_ref: options.handler_ref,
      handler_digest: options.handler_digest,
      input_type_ref: options.result_type_ref,
      output_type_ref: options.result_type_ref,
      return_mode: options.return_mode ?? "replace_result",
    });
  }
}
