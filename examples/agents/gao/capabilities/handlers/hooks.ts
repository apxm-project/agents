import {
  hook,
  HookMode,
  LifecycleEvent,
  type HookCall,
  type HookContext,
} from "@apxm/frontend";

import { prompt, renderStudioContextSupplement, SUMMARY_KEY } from "./context.js";

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
})((ctx: HookContext) => {
  const context = [
    prompt("terminology"),
    prompt("workflow_authoring"),
    `Recent context:\n${ctx.recall_window(4)}`,
    `Running summary:\n${String(ctx.recall(SUMMARY_KEY) ?? "")}`,
  ].join("\n\n");
  return ctx.prependSystem(context);
});

export const gate_compose_workflow = hook({
  on: LifecycleEvent.PRE_CAP,
  match: "compose_workflow",
  mode: HookMode.GATE,
})((_ctx: HookContext, call: HookCall) => {
  const name = String(call.args.name ?? "").trim();
  if (!name) {
    return _ctx.deny("workflow name is required");
  }
  return _ctx.allow();
});

export const redact_tool_results = hook({
  on: LifecycleEvent.POST_CAP,
  match: "*",
  mode: HookMode.OBSERVE,
})((ctx: HookContext, _call: HookCall, result: unknown) => {
  let text = String(result ?? "");
  for (const marker of ["api_key=", "token=", "secret="]) {
    text = text.replaceAll(marker, `${marker}<redacted>`);
  }
  return ctx.replaceResult(text);
});

export const compact_conversation = hook({
  on: LifecycleEvent.POST_TURN,
  mode: HookMode.OBSERVE,
})((ctx: HookContext) => {
  const window = ctx.recall_window(40);
  if (ctx.count_tokens(window) < 20_000) {
    return null;
  }
  const summary = ctx.ask(
    "Update Gao's running summary. Preserve user goals, workflow decisions, APXM terms, tool grants, and open questions.\n\n" +
      window,
    "You maintain a compact APXM workflow-design session summary.",
  );
  ctx.umem(SUMMARY_KEY, summary);
  return null;
});
