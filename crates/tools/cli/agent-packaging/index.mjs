// Builds TypeScript handler descriptors; the Rust runtime owns their execution contract.

import { createHash } from "node:crypto";

const ANSWER_KIND = "apxm.tool-answer";
const FIELD_KIND = Symbol("apxm.tool-field");
const INPUT_KIND = Symbol("apxm.tool-input");

function handlerModule() {
  return process.env.APXM_HANDLER_MODULE ?? "__unknown__";
}

function handlerId(module, qualname) {
  const digest = createHash("sha256")
    .update(`${module}:${qualname}`, "utf8")
    .digest("hex");
  return `sha256:${digest}`;
}

function mark(value, kind) {
  Object.defineProperty(value, kind, { value: true });
  return value;
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function wrapTool(fn, definition) {
  const module = handlerModule();
  const qualname = fn.name || "tool";
  return {
    kind: "tool",
    name: definition.name,
    description: definition.description,
    schema: definition.input,
    handler_id: handlerId(module, qualname),
    module,
    qualname,
    fn,
  };
}

function text(options = {}) {
  return mark({
    type: "string",
    ...(options.minLength === undefined ? {} : { minLength: options.minLength }),
  }, FIELD_KIND);
}

function object(fields) {
  const entries = Object.entries(fields);
  if (entries.length === 0) {
    throw new TypeError("Tool.object requires at least one field");
  }
  if (entries.some(([, field]) => field?.[FIELD_KIND] !== true)) {
    throw new TypeError("Tool.object fields must come from Tool helpers");
  }
  return mark({
    type: "object",
    properties: Object.fromEntries(entries),
    required: entries.map(([name]) => name),
    additionalProperties: false,
  }, INPUT_KIND);
}

function answer(value) {
  if (!isPlainObject(value)) {
    throw new TypeError("Tool.answer requires one plain answer object");
  }
  return { kind: ANSWER_KIND, value };
}

function define(definition) {
  if (typeof definition !== "object" || definition === null) {
    throw new TypeError("Tool.define requires one definition object");
  }
  if (typeof definition.name !== "string" || definition.name.length === 0) {
    throw new TypeError("Tool.define requires a non-empty name");
  }
  if (typeof definition.description !== "string" || definition.description.length === 0) {
    throw new TypeError("Tool.define requires a non-empty description");
  }
  if (definition.input?.[INPUT_KIND] !== true) {
    throw new TypeError("Tool.define input must come from Tool.object");
  }
  if (typeof definition.run !== "function") {
    throw new TypeError("Tool.define requires a run function");
  }
  return wrapTool(definition.run, definition);
}

/** The only TypeScript package-handler authoring object. */
export const Tool = Object.freeze({ define, object, text, answer });

/** Identify a declared TypeScript Capability handler. */
export function isFunctionTool(value) {
  return (
    typeof value === "object" &&
    value !== null &&
    value.kind === "tool" &&
    typeof value.handler_id === "string" &&
    typeof value.fn === "function"
  );
}

/** Recognize the internal answer envelope emitted by Tool.answer. */
export function isToolAnswer(value) {
  return (
    isPlainObject(value) &&
    value.kind === ANSWER_KIND &&
    isPlainObject(value.value)
  );
}

/** Produce a stable handler identity for a package module export. */
export function makeHandlerId(module, qualname) {
  return handlerId(module, qualname);
}
