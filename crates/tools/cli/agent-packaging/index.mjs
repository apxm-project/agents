// Builds TypeScript handler descriptors; the Rust runtime owns their execution contract.
//
// Two inversions make a declaration hard to misuse. `required` lives on the
// property it describes, so no sibling list can fall out of step with the
// properties it names. `additionalProperties` is stated rather than defaulted,
// because whether a Capability accepts arguments it never declared is a decision
// and an unstated decision is the permissive one exactly when that is worst.
//
// `Tool.define` returns the Capability id: the reference and the implementation
// are one object, so referencing one Capability while implementing another is
// not something this surface can express.

import { createHash } from "node:crypto";

const ANSWER_KIND = "apxm.tool-answer";
const PROPERTY_KIND = Symbol("apxm.tool-property");
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

function property(required, constraints) {
  if (typeof required !== "boolean") {
    throw new TypeError("a Tool property states whether it is required");
  }
  const declared = mark({ ...constraints }, PROPERTY_KIND);
  Object.defineProperty(declared, "required", { value: required });
  return declared;
}

function text({ required, minLength, description } = {}) {
  return property(required, {
    type: "string",
    ...(minLength === undefined ? {} : { minLength }),
    ...(description === undefined ? {} : { description }),
  });
}

function integer({ required, minimum, description } = {}) {
  return property(required, {
    type: "integer",
    ...(minimum === undefined ? {} : { minimum }),
    ...(description === undefined ? {} : { description }),
  });
}

function object(schema) {
  if (!isPlainObject(schema)) {
    throw new TypeError("Tool.object takes one schema declaration");
  }
  if (typeof schema.additionalProperties !== "boolean") {
    throw new TypeError(
      "Tool.object states whether the Capability accepts undeclared arguments",
    );
  }
  const entries = Object.entries(schema.properties ?? {});
  if (entries.length === 0) {
    throw new TypeError("Tool.object requires at least one declared argument");
  }
  if (entries.some(([, declared]) => declared?.[PROPERTY_KIND] !== true)) {
    throw new TypeError("Tool.object properties come from Tool helpers");
  }
  return mark({
    type: "object",
    properties: Object.fromEntries(
      entries.map(([name, declared]) => [name, { ...declared }]),
    ),
    // Derived from the properties, never authored beside them.
    required: entries.filter(([, declared]) => declared.required).map(([name]) => name),
    additionalProperties: schema.additionalProperties,
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
  if (typeof definition.readOnly !== "boolean") {
    throw new TypeError(
      "Tool.define states whether the Capability mutates state outside itself",
    );
  }
  if (definition.input?.[INPUT_KIND] !== true) {
    throw new TypeError("Tool.define input must come from Tool.object");
  }
  if (typeof definition.run !== "function") {
    throw new TypeError("Tool.define requires a run function");
  }
  const module = handlerModule();
  const qualname = definition.run.name || "tool";
  return {
    kind: "tool",
    capabilityId: definition.name,
    name: definition.name,
    description: definition.description,
    read_only: definition.readOnly,
    schema: definition.input,
    handler_id: handlerId(module, qualname),
    module,
    qualname,
    fn: definition.run,
  };
}

/** The only TypeScript package-handler authoring object. */
export const Tool = Object.freeze({ define, object, text, integer, answer });

/** Identify a declared TypeScript Capability handler. */
export function isFunctionTool(value) {
  return (
    typeof value === "object" &&
    value !== null &&
    value.kind === "tool" &&
    typeof value.handler_id === "string" &&
    typeof value.capabilityId === "string" &&
    typeof value.read_only === "boolean" &&
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
