import { createHash } from "node:crypto";

import type { ToolFn } from "./types.js";

/** Module-level registry — the tool worker resolves handlers by id. */
export const TOOL_REGISTRY = new Map<string, ToolFn>();

/** Stable handler id from `module:qualname`. */
export function makeHandlerId(module: string, qualname: string): string {
  const key = `${module}:${qualname}`;
  const digest = createHash("sha256").update(key, "utf8").digest("hex");
  return `sha256:${digest}`;
}

/** Module name injected by {@link compileHandlers} while loading entry files. */
export function getHandlerModule(): string {
  return process.env.APXM_HANDLER_MODULE ?? "__unknown__";
}
