/** Static Hook bindings and the portable Agent Facade typing surface. */

import type { HookBinding } from "./agent-program.ts";

export type AgentFacade<C> = {
  context: C;
};

export type HookHandler<C, T> = (agent: AgentFacade<C>) => T | Promise<T>;

export class Hook {
  readonly hookId: string;
  readonly scope: HookBinding["scope"];
  readonly phase: HookBinding["phase"];
  readonly targetSelector: string;
  readonly declarationOrder: number;
  readonly handlerRef: string;
  readonly handlerDigest: string;
  readonly inputTypeRef: string;
  readonly outputTypeRef: string;
  readonly returnMode: HookBinding["return_mode"];

  constructor(fields: HookBinding) {
    this.hookId = fields.hook_id;
    this.scope = fields.scope;
    this.phase = fields.phase;
    this.targetSelector = fields.target_selector;
    this.declarationOrder = fields.declaration_order;
    this.handlerRef = fields.handler_ref;
    this.handlerDigest = fields.handler_digest;
    this.inputTypeRef = fields.input_type_ref;
    this.outputTypeRef = fields.output_type_ref;
    this.returnMode = fields.return_mode;
  }

  toBinding(): HookBinding {
    return {
      hook_id: this.hookId,
      scope: this.scope,
      phase: this.phase,
      target_selector: this.targetSelector,
      declaration_order: this.declarationOrder,
      handler_ref: this.handlerRef,
      handler_digest: this.handlerDigest,
      input_type_ref: this.inputTypeRef,
      output_type_ref: this.outputTypeRef,
      return_mode: this.returnMode,
    };
  }

  static beforeLoop(opts: {
    hook_id: string;
    target_selector: string;
    handler_ref: string;
    handler_digest: string;
    context_type_ref: string;
    declaration_order?: number;
  }): Hook {
    return new Hook({
      hook_id: opts.hook_id,
      scope: "loop",
      phase: "before",
      target_selector: opts.target_selector,
      declaration_order: opts.declaration_order ?? 0,
      handler_ref: opts.handler_ref,
      handler_digest: opts.handler_digest,
      input_type_ref: opts.context_type_ref,
      output_type_ref: opts.context_type_ref,
      return_mode: "observe",
    });
  }

  static afterModel(opts: {
    hook_id: string;
    target_selector: string;
    handler_ref: string;
    handler_digest: string;
    result_type_ref: string;
    declaration_order?: number;
    return_mode?: HookBinding["return_mode"];
  }): Hook {
    return new Hook({
      hook_id: opts.hook_id,
      scope: "model",
      phase: "after",
      target_selector: opts.target_selector,
      declaration_order: opts.declaration_order ?? 0,
      handler_ref: opts.handler_ref,
      handler_digest: opts.handler_digest,
      input_type_ref: opts.result_type_ref,
      output_type_ref: opts.result_type_ref,
      return_mode: opts.return_mode ?? "replace_result",
    });
  }
}
