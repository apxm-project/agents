// Defines Gao's typed lifecycle hooks for context, policy, and memory.
import {
  hook,
  HookMode,
  LifecycleEvent,
  type HookCall,
  type HookContext,
} from "@apxm/frontend";

const SUMMARY_KEY = "gao:conversation:summary";
const SUMMARY_OUTPUT_TOKEN_LIMIT = 1_024;
const SENSITIVE_KEY =
  /(?:api[_-]?key|token|secret|password|credential|authorization|auth)/i;
const ASSIGNMENT_SECRET =
  /(\b(?:api[_-]?key|access[_-]?token|refresh[_-]?token|token|secret|password|credential|authorization|auth)\b\s*[:=]\s*)(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\s,;}\]\r\n]+)/gi;
const BEARER_SECRET = /\bbearer\s+[^\s,;}\]]+/gi;

function scrubText(value: string): string {
  return value
    .replace(BEARER_SECRET, "<redacted>")
    .replace(ASSIGNMENT_SECRET, "$1<redacted>");
}

function scrubSecrets(
  value: unknown,
  options: { maxStringChars?: number } = {},
  key = "",
): unknown {
  if (key && SENSITIVE_KEY.test(key)) {
    return "<redacted>";
  }
  if (typeof value === "string") {
    const scrubbed = scrubText(value);
    const maxStringChars = options.maxStringChars;
    return maxStringChars != null && scrubbed.length > maxStringChars
      ? `${scrubbed.slice(0, maxStringChars)}…`
      : scrubbed;
  }
  if (Array.isArray(value)) {
    return value.map((item) => scrubSecrets(item, options));
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([entryKey, entryValue]) => [
        entryKey,
        scrubSecrets(entryValue, options, entryKey),
      ]),
    );
  }
  return value;
}

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
    SUMMARY_OUTPUT_TOKEN_LIMIT,
    "You maintain a compact APXM workflow-design session summary.",
  );
  ctx.umem(SUMMARY_KEY, summary);
  return null;
});
