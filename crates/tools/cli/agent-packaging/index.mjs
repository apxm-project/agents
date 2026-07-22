// Defines TypeScript handler descriptors owned by agent packaging.

import { createHash } from "node:crypto";

function handlerModule() {
  return process.env.APXM_HANDLER_MODULE ?? "__unknown__";
}

function handlerId(module, qualname) {
  const digest = createHash("sha256")
    .update(`${module}:${qualname}`, "utf8")
    .digest("hex");
  return `sha256:${digest}`;
}

function wrapTool(fn, options = {}) {
  const module = handlerModule();
  const qualname = fn.name || "tool";
  return {
    kind: "tool",
    name: options.name ?? qualname,
    description: options.description ?? "",
    schema: options.schema ?? {},
    handler_id: handlerId(module, qualname),
    module,
    qualname,
    fn,
  };
}

/** Declare a package-local TypeScript Capability handler. */
export function tool(fnOrOptions = {}) {
  if (typeof fnOrOptions === "function") {
    return wrapTool(fnOrOptions);
  }
  return (fn) => wrapTool(fn, fnOrOptions);
}

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

/** Produce a stable handler identity for a package module export. */
export function makeHandlerId(module, qualname) {
  return handlerId(module, qualname);
}
