import { hook, HookMode, LifecycleEvent, type HookCall, type HookContext } from "@apxm/frontend";

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
