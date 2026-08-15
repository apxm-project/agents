// Type-level use of the package's public surface. `index.d.ts` alone would
// typecheck as long as it parses; exercising it here is what makes the gate
// able to fail when a declaration stops describing what an author writes.
//
// Nothing runs: `tsc --noEmit` is the whole assertion.

import { Tool, isFunctionTool, isToolAnswer, makeHandlerId } from "../index.js";
import type { CapabilityId, ToolAnswer } from "../index.js";

const echo = Tool.define({
  name: "echo",
  description: "Return an input message without executing a runtime effect.",
  readOnly: true,
  input: Tool.object({
    additionalProperties: false,
    properties: {
      message: Tool.text({ required: true, minLength: 1 }),
      repeat: Tool.integer({ required: false, minimum: 1 }),
    },
  }),
  run({ message, repeat }) {
    // A required property arrives as its declared type; an optional one arrives
    // possibly undefined. Both facts come from the schema literal alone.
    const widened: string = message;
    const optional: number | undefined = repeat;
    return Tool.answer({ text: widened, times: optional ?? 1 });
  },
});

// The declaration keeps the property types it derived.
const declared: CapabilityId<{
  message: ReturnType<typeof Tool.text<true>>;
  repeat: ReturnType<typeof Tool.integer<false>>;
}> = echo;
const handlerId: string = declared.handler_id;
const sameId: string = makeHandlerId(declared.module, declared.qualname);

// @ts-expect-error a declared-required argument cannot be left out
const missingRequired: Parameters<typeof declared.fn>[0] = { repeat: 2 };

// @ts-expect-error an answer carries an object value, never a bare string
const badAnswer: ToolAnswer = Tool.answer("not an object");

const narrowed: boolean = isFunctionTool(echo) && isToolAnswer(Tool.answer({ ok: true }));

export type Surface = [typeof handlerId, typeof sameId, typeof missingRequired,
  typeof badAnswer, typeof narrowed];
