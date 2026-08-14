// Verifies the package-local Tool definition API without a runtime worker.

import assert from "node:assert/strict";
import test from "node:test";

import { Tool, isFunctionTool, isToolAnswer } from "./index.mjs";

test("Tool.define generates a closed input schema and typed answer", () => {
  const Echo = Tool.define({
    name: "echo",
    description: "Return an input message without executing a runtime effect.",
    readOnly: true,
    input: Tool.object({
      additionalProperties: false,
      properties: {
        message: Tool.text({ required: true, minLength: 1 }),
        note: Tool.text({ required: false }),
      },
    }),
    run({ message }) {
      return Tool.answer({ message });
    },
  });

  assert.equal(isFunctionTool(Echo), true);
  // The reference and the implementation are one object.
  assert.equal(Echo.capabilityId, "echo");
  assert.equal(Echo.read_only, true);
  // `required` is derived from the properties, so the two cannot disagree.
  assert.deepEqual(Echo.schema, {
    type: "object",
    properties: {
      message: { type: "string", minLength: 1 },
      note: { type: "string" },
    },
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
      readOnly: true,
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


test("a handler states read-only and openness rather than defaulting either", () => {
  const declaration = (overrides) => ({
    name: "stated",
    description: "A handler that must state its own decisions.",
    readOnly: false,
    input: Tool.object({
      additionalProperties: false,
      properties: { message: Tool.text({ required: true }) },
    }),
    run() {
      return Tool.answer({ ok: true });
    },
    ...overrides,
  });

  // An unstated read-only decision is not defaulted to the safe answer; it is
  // refused, because a handler nobody asked is a handler nobody can trust.
  assert.throws(
    () => Tool.define({ ...declaration(), readOnly: undefined }),
    /mutates state outside itself/,
  );
  // The same for an object that would silently accept undeclared arguments.
  assert.throws(
    () => Tool.object({ properties: { message: Tool.text({ required: true }) } }),
    /undeclared arguments/,
  );
  // A property that does not say whether it is required is not a property.
  assert.throws(() => Tool.text({ minLength: 1 }), /states whether it is required/);
  assert.equal(Tool.define(declaration()).read_only, false);
});
