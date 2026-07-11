import { hook, HookMode, LifecycleEvent, type HookCall, type HookContext } from "@apxm/frontend";

import { context_prompt, SUMMARY_KEY } from "./context.js";

export const inject_context = hook({
  on: LifecycleEvent.PRE_TURN,
  mode: HookMode.OBSERVE,
})(async (ctx: HookContext) => {
  const recent = await ctx.recallWindow(6);
  const summary = String((await ctx.recall(SUMMARY_KEY)) ?? "");
  return ctx.prependSystem(context_prompt(summary, recent));
});

export const read_back_summary = hook({
  on: LifecycleEvent.PRE_ASK,
  mode: HookMode.OBSERVE,
})(async (ctx: HookContext) => {
  const summary = String((await ctx.recall(SUMMARY_KEY)) ?? "");
  if (!summary.trim()) {
    return null;
  }
  return ctx.prependSystem(`Conversation summary:\n${summary}`);
});

export const redact_tool_results = hook({
  on: LifecycleEvent.POST_CAP,
  match: "*",
  mode: HookMode.OBSERVE,
})((ctx: HookContext, _call: HookCall, result: unknown) => {
  let text = String(result ?? "");
  for (const marker of ["api_key=", "token=", "secret=", "password="]) {
    text = text.replaceAll(marker, `${marker}<redacted>`);
  }
  return ctx.replaceResult(text);
});
