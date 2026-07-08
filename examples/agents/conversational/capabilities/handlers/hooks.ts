import { hook, HookMode, LifecycleEvent, type HookCall, type HookContext } from "@apxm/frontend";

import { context_prompt, SUMMARY_KEY } from "./context.js";

export const inject_context = hook({
  on: LifecycleEvent.PRE_TURN,
  mode: HookMode.OBSERVE,
})((ctx: HookContext) => {
  const recent = ctx.recall_window(6);
  const summary = String(ctx.recall(SUMMARY_KEY) ?? "");
  return ctx.prependSystem(context_prompt(summary, recent));
});

export const read_back_summary = hook({
  on: LifecycleEvent.PRE_ASK,
  mode: HookMode.OBSERVE,
})((ctx: HookContext) => {
  const summary = String(ctx.recall(SUMMARY_KEY) ?? "");
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

export const compact_conversation = hook({
  on: LifecycleEvent.POST_TURN,
  mode: HookMode.OBSERVE,
})((ctx: HookContext) => {
  const window = ctx.recall_window(40);
  if (ctx.count_tokens(window) < 12_000) {
    return null;
  }
  const summary = ctx.ask(
    `Update the running conversation summary. Keep durable user goals, decisions, constraints, and open questions.\n\n${window}`,
    "You maintain compact conversational memory.",
  );
  ctx.umem(SUMMARY_KEY, summary);
  return null;
});
