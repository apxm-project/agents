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

// Bounded-context compaction is now the runtime default
// (`[runtime] compaction_policy` in agent.toml), not a hand-rolled
// `post_turn` hook. The retired version called `ctx.count_tokens`/`ctx.ask`
// itself on every turn (the same DIY pattern as
// examples/python/conversational/controllable_agent.py's earlier hand-rolled shape);
// the runtime mechanism reuses the shipped bpe estimator and emits
// `context_window_warning`/`context_compacted` events instead of folding
// silently. See `examples/agents/conversational-opt-out` for the variant
// with `compaction_policy` entirely absent (the opt-out dial).
