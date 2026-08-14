// Verifies the package-local Tool definition API without a runtime worker.

import assert from "node:assert/strict";
import test from "node:test";

import { Tool, isFunctionTool, isToolAnswer } from "./index.mjs";

test("Tool.define generates a closed input schema and typed answer", () => {
  const Echo = Tool.define({
    name: "echo",
    description: "Return an input message without executing a runtime effect.",
    input: Tool.object({ message: Tool.text({ minLength: 1 }) }),
    run({ message }) {
      return Tool.answer({ message });
    },
  });

  assert.equal(isFunctionTool(Echo), true);
  assert.deepEqual(Echo.schema, {
    type: "object",
    properties: { message: { type: "string", minLength: 1 } },
    required: ["message"],
    additionalProperties: false,
  });
  assert.deepEqual(Echo.fn({ message: "hello" }), {
    kind: "apxm.tool-answer",
    value: { message: "hello" },
  });
  assert.equal(isToolAnswer(Echo.fn({ message: "hello" })), true);
});

test("Tool.define rejects raw schema and an untyped answer", () => {
  assert.throws(
    () => Tool.define({
      name: "raw",
      description: "Invalid raw schema fixture.",
      input: { type: "object", properties: {}, required: [], additionalProperties: false },
      run() {
        return Tool.answer({ value: "unused" });
      },
    }),
    /Tool\.object/,
  );
  assert.equal(isToolAnswer({ value: { message: "plain" } }), false);
  assert.throws(() => Tool.answer(new Date()), /plain answer object/);
  assert.equal(
    isToolAnswer({ kind: "apxm.tool-answer", value: new Date() }),
    false,
  );
});
