import { createHash } from "node:crypto";
import { describe, expect, expectTypeOf, it } from "vitest";

import {
  GATE_LIFECYCLE_EVENTS,
  HookMode,
  LifecycleEvent,
  type HookCall,
  type HookContext,
  type JsonSchema,
  hook,
  makeHandlerId,
  tool,
} from "../src/handlers/index.js";

describe("makeHandlerId", () => {
  it("derives a stable sha256 id from module:qualname", () => {
    const id = makeHandlerId("examples.handlers", "search_docs");
    const expected = `sha256:${createHash("sha256")
      .update("examples.handlers:search_docs", "utf8")
      .digest("hex")}`;

    expect(id).toBe(expected);
    expect(makeHandlerId("examples.handlers", "search_docs")).toBe(id);
    expect(id).toMatch(/^sha256:[a-f0-9]{64}$/);
  });
});

describe("tool metadata", () => {
  it("preserves the authored argument schema", () => {
    interface PlanArgs {
      request: string;
    }
    const schema: JsonSchema = {
      type: "object",
      properties: { request: { type: "string" } },
      required: ["request"],
      additionalProperties: false,
    };
    const wrapped = tool({ name: "plan_workflow", schema })((args: PlanArgs) => args.request);
    expect(wrapped.schema).toEqual(schema);
    expect(wrapped.fn({ request: "typed" })).toBe("typed");
  });
});

describe("hook validation", () => {
  it("keeps hook context data-only for model instructions", () => {
    type NoPromptPrepend = "prependSystem" extends keyof HookContext ? false : true;
    type NoPromptReplace = "setSystem" extends keyof HookContext ? false : true;

    expectTypeOf<NoPromptPrepend>().toEqualTypeOf<true>();
    expectTypeOf<NoPromptReplace>().toEqualTypeOf<true>();
  });

  it("accepts typed lifecycle hook arguments", () => {
    const wrapped = hook({ on: LifecycleEvent.PRE_CAP, mode: HookMode.GATE })(
      (ctx: HookContext, call: HookCall) =>
        call.name === "write" ? ctx.deny("write requires approval") : ctx.allow(),
    );
    expect(wrapped.event).toBe("pre_cap");
  });

  it("accepts observe mode on post_turn", () => {
    const wrapped = hook({ on: LifecycleEvent.POST_TURN, mode: HookMode.OBSERVE })(() => ({
      decision: "allow",
    }));
    expect(wrapped.event).toBe("post_turn");
    expect(wrapped.mode).toBe("observe");
  });

  it("rejects unknown lifecycle events", () => {
    expect(() =>
      hook({ on: "not_an_event", mode: HookMode.OBSERVE })(() => ({ decision: "allow" })),
    ).toThrow(/not a valid event/);
  });

  it("rejects unknown hook modes", () => {
    expect(() =>
      hook({ on: LifecycleEvent.PRE_TURN, mode: "block" as HookMode })(() => ({
        decision: "allow",
      })),
    ).toThrow(/invalid; expected one of/);
  });

  it("rejects gate mode on post-execution events", () => {
    expect(() =>
      hook({ on: LifecycleEvent.POST_TURN, mode: HookMode.GATE })(() => ({
        decision: "allow",
      })),
    ).toThrow(/gate mode is only permitted/);
  });

  it("allows gate mode on pre-execution events", () => {
    for (const event of GATE_LIFECYCLE_EVENTS) {
      const wrapped = hook({ on: event, mode: HookMode.GATE })(() => ({ decision: "allow" }));
      expect(wrapped.mode).toBe("gate");
      expect(wrapped.event).toBe(event);
    }
  });
});
