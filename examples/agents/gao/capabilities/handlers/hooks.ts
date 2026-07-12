// Defines Gao's typed lifecycle hooks for context, policy, and memory.
import {
  hook,
  HookMode,
  LifecycleEvent,
  type HookCall,
  type HookContext,
  type HookDecision,
} from "@apxm/frontend";

import {
  prompt,
  renderStudioContextSupplement,
  scrubSecrets,
  SUMMARY_KEY,
} from "./context.js";

declare module "@apxm/frontend" {
  interface HookContext {
    defer(): HookDecision;
  }
}

export const inject_context = hook({
  on: LifecycleEvent.PRE_TURN,
  mode: HookMode.OBSERVE,
})(async (ctx: HookContext) => {
  const supplement = await renderStudioContextSupplement(ctx, ctx.context);
  return ctx.prependSystem(supplement);
});

export const inject_apxm_context = hook({
  on: LifecycleEvent.PRE_ASK,
  mode: HookMode.OBSERVE,
})(async (ctx: HookContext) => {
  const recent = await ctx.recallWindow(4);
  const summary = await ctx.recall(SUMMARY_KEY);
  const context = [
    prompt("terminology"),
    prompt("workflow_authoring"),
    `Recent context:\n${recent}`,
    `Running summary:\n${String(summary ?? "")}`,
  ].join("\n\n");
  return ctx.prependSystem(context);
});

export const gate_compose_workflow = hook({
  on: LifecycleEvent.PRE_CAP,
  match: "compose_workflow",
  mode: HookMode.GATE,
})((ctx: HookContext, call: HookCall) => {
  const name = String(call.args.name ?? "").trim();
  if (!name) {
    return ctx.deny("compose_workflow requires a non-empty workflow name");
  }
  const air = String(call.args.air ?? "").trim();
  if (!air) {
    return ctx.deny("compose_workflow requires non-empty AIR source");
  }
  return ctx.defer();
});

export const redact_tool_results = hook({
  on: LifecycleEvent.POST_CAP,
  match: "*",
  mode: HookMode.OBSERVE,
})((ctx: HookContext, _call: HookCall, result: unknown) => {
  return ctx.replaceResult(scrubSecrets(result));
});

export const compact_conversation = hook({
  on: LifecycleEvent.POST_TURN,
  mode: HookMode.OBSERVE,
})(async (ctx: HookContext) => {
  const window = await ctx.recallWindow(40);
  if ((await ctx.countTokens(window)) < 20_000) {
    return null;
  }
  const summary = await ctx.ask(
    "Update Gao's running summary. Preserve user goals, workflow decisions, APXM terms, tool grants, and open questions.\n\n" +
      window,
    "You maintain a compact APXM workflow-design session summary.",
  );
  ctx.umem(SUMMARY_KEY, summary);
  return null;
});
