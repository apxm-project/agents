import { hook, HookMode, LifecycleEvent, type HookContext } from "@apxm/frontend";

const CHECKPOINT_KEY = "coder:checkpoint";

export const restore_session = hook({
  on: LifecycleEvent.SESSION_START,
  mode: HookMode.OBSERVE,
})((ctx: HookContext) => {
  const checkpoint = ctx.recall(CHECKPOINT_KEY);
  if (!checkpoint) {
    return null;
  }
  return ctx.prependSystem(
    `Continue from the saved coding checkpoint: ${String(checkpoint)}. Re-read the workspace before acting.`,
  );
});
