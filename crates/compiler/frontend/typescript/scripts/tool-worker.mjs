#!/usr/bin/env node
/**
 * NDJSON RPC subprocess worker for TypeScript tool/hook dispatch.
 *
 * Invoked as: node tool-worker.mjs <manifest.json>
 *
 * Wire protocol matches apxm/tool_worker.py (call, host_call/host_result).
 */

import { createInterface } from "node:readline";
import { existsSync, readFileSync } from "node:fs";
import * as nodeModule from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const WIRE_VERSION = 1;
const HOOK_PAYLOAD_KEY = "__apxm_hook__";
const HOST_CALL_TIMEOUT_MS = 120_000;

/** @type {Map<string, Function>} */
const registry = new Map();

/** @type {Map<string, import('node:readline').Interface>} */
const hostPending = new Map();
let hostCallCounter = 0;

function installApxmFrontendResolver() {
  if (typeof nodeModule.registerHooks !== "function") return;

  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const frontendDist = path.resolve(scriptDir, "../dist/index.js");
  if (!existsSync(frontendDist)) return;
  const frontendUrl = pathToFileURL(frontendDist).href;

  nodeModule.registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "@apxm/frontend") {
        return { url: frontendUrl, shortCircuit: true };
      }
      if (
        specifier.startsWith(".") &&
        specifier.endsWith(".js") &&
        context.parentURL?.startsWith("file:")
      ) {
        const parentDir = path.dirname(fileURLToPath(context.parentURL));
        const tsPath = path.resolve(
          parentDir,
          `${specifier.slice(0, -".js".length)}.ts`,
        );
        if (existsSync(tsPath)) {
          return { url: pathToFileURL(tsPath).href, shortCircuit: true };
        }
      }
      return nextResolve(specifier, context);
    },
  });
}

installApxmFrontendResolver();

function emitLine(obj) {
  process.stdout.write(`${JSON.stringify(obj)}\n`);
}

function logWorker(level, message) {
  process.stderr.write(`${JSON.stringify({ v: WIRE_VERSION, type: "log", level, message })}\n`);
}

async function loadManifest(manifestPath) {
  const entries = JSON.parse(readFileSync(manifestPath, "utf8"));
  await Promise.all(
    entries.map(async (entry) => {
      const sourceFile = entry.source_file;
      if (!sourceFile) {
        logWorker("error", `manifest entry ${entry.handler_id} missing source_file`);
        return;
      }
      const absPath = path.resolve(sourceFile);
      try {
        const moduleUrl = pathToFileURL(absPath);
        moduleUrl.searchParams.set("t", String(Date.now()));
        const mod = await import(moduleUrl.href);
        const qualParts = String(entry.qualname || entry.name).split(".");
        let target = mod;
        for (const part of qualParts) {
          if (part === "<locals>") {
            throw new Error(`cannot resolve local qualname ${entry.qualname}`);
          }
          target = target?.[part];
        }
        if (target && typeof target === "object" && typeof target.fn === "function") {
          target = target.fn;
        }
        if (typeof target !== "function") {
          throw new Error(`handler ${entry.handler_id} not found in ${absPath}`);
        }
        if (!registry.has(entry.handler_id)) {
          registry.set(entry.handler_id, target);
        }
      } catch (err) {
        logWorker(
          "error",
          `failed to import ${sourceFile}: ${err instanceof Error ? err.message : err}`,
        );
      }
    }),
  );
}

function deliverHostResult(msg) {
  const reqId = msg.req_id;
  const waiter = hostPending.get(reqId);
  if (!waiter) return;
  hostPending.delete(reqId);
  waiter.resolve(msg);
}

class HookCall {
  constructor(name, args) {
    this.name = name;
    this.args = args || {};
  }
}

class HookCtx {
  constructor(payload, parentReqId = "") {
    this.remaining_budget = payload.remaining_budget;
    this.context = payload.context;
    this._reqId = parentReqId;
    this._system = payload.system || "";
    this._writes = [];
  }

  log(...args) {
    console.error("[hook]", ...args);
  }

  allow() {
    return { decision: "allow" };
  }

  deny(reason = "") {
    return { decision: "deny", reason };
  }

  edit_args(args) {
    return { decision: "edit_args", args };
  }

  editArgs(args) {
    return this.edit_args(args);
  }

  replace_result(result) {
    return { decision: "replace_result", result };
  }

  replaceResult(result) {
    return this.replace_result(result);
  }

  prepend_system(text) {
    return { decision: "prepend_system", text };
  }

  prependSystem(text) {
    return this.prepend_system(text);
  }

  set_system(text) {
    return { decision: "set_system", text };
  }

  setSystem(text) {
    return this.set_system(text);
  }

  read_agents_md() {
    return "";
  }

  readAgentsMd() {
    return this.read_agents_md();
  }

  recall_window(_n = 4, _prefix = "conversation:") {
    return "";
  }

  recallWindow(n = 4, prefix = "conversation:") {
    return this.recall_window(n, prefix);
  }

  umem(key, value) {
    this._writes.push({ key, value });
  }

  _hostCall(method, params) {
    if (!this._reqId) {
      throw new Error(`ctx host call '${method}' unavailable: no parent request bound`);
    }
    const cbId = `h-${++hostCallCounter}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        hostPending.delete(cbId);
        reject(new Error(`ctx host call '${method}' timed out`));
      }, HOST_CALL_TIMEOUT_MS);

      hostPending.set(cbId, {
        resolve: (msg) => {
          clearTimeout(timer);
          if (msg.ok) {
            resolve(msg.value);
          } else {
            const message = msg.error?.message || "unknown error";
            reject(new Error(`ctx host call '${method}' failed: ${message}`));
          }
        },
      });

      emitLine({
        v: WIRE_VERSION,
        type: "host_call",
        req_id: cbId,
        parent_req_id: this._reqId,
        method,
        params,
      });
    });
  }

  ask(prompt, system = null) {
    const params = { prompt };
    if (system != null) params.system = system;
    const value = this._hostCall("llm.ask", params);
    return value.then((v) => (typeof v === "string" ? v : String(v)));
  }

  call(name, args = {}) {
    return this._hostCall("tool.call", { name, args });
  }

  count_tokens(text) {
    return this.call("count_tokens", { text }).then((value) => {
      if (typeof value === "number") return value;
      if (typeof value === "string") return Number.parseInt(value, 10) || 0;
      return 0;
    });
  }

  countTokens(text) {
    return this.count_tokens(text);
  }

  recall(_key) {
    return null;
  }
}

function invokeHook(fn, event, payload, reqId) {
  const ctx = new HookCtx(payload, reqId);
  let ret;
  if (event === "pre_cap") {
    const call = payload.call || {};
    ret = fn(ctx, new HookCall(call.name || "", call.args || {}));
  } else if (event === "post_cap") {
    const call = payload.call || {};
    ret = fn(ctx, new HookCall(call.name || "", {}), payload.result);
  } else if (event === "post_ask" || event === "post_turn") {
    ret = fn(ctx, payload.reply);
  } else {
    ret = fn(ctx);
  }
  const decision = ret && typeof ret === "object" ? ret : {};
  if (ctx._writes.length > 0 && decision.writes === undefined) {
    return { ...decision, writes: ctx._writes };
  }
  return decision;
}

async function handleHookCall(reqId, fn, payload) {
  if (typeof fn !== "function") {
    emitLine({
      v: WIRE_VERSION,
      type: "result",
      req_id: reqId,
      ok: false,
      error: {
        kind: "unknown_handler",
        message: "no handler registered for hook",
        traceback: "",
      },
    });
    return;
  }
  try {
    const event = payload.event || "";
    const value = await Promise.resolve(invokeHook(fn, event, payload, reqId));
    emitLine({
      v: WIRE_VERSION,
      type: "result",
      req_id: reqId,
      ok: true,
      value: value && typeof value === "object" ? value : { decision: "allow" },
    });
  } catch (err) {
    emitLine({
      v: WIRE_VERSION,
      type: "result",
      req_id: reqId,
      ok: false,
      error: {
        kind: err instanceof Error ? err.name : "Error",
        message: err instanceof Error ? err.message : String(err),
        traceback: err instanceof Error ? err.stack || "" : "",
      },
    });
  }
}

async function handleCall(msg) {
  const reqId = msg.req_id;
  const toolId = msg.tool_id;
  const args = msg.args || {};
  const fn = registry.get(toolId);

  if (args && typeof args === "object" && HOOK_PAYLOAD_KEY in args) {
    await handleHookCall(reqId, fn, args[HOOK_PAYLOAD_KEY]);
    return;
  }

  if (typeof fn !== "function") {
    emitLine({
      v: WIRE_VERSION,
      type: "result",
      req_id: reqId,
      ok: false,
      error: {
        kind: "unknown_handler",
        message: `no handler registered for tool_id=${JSON.stringify(toolId)}`,
        traceback: "",
      },
    });
    return;
  }

  const deadlineMs = msg.deadline_ms;
  try {
    const run = Promise.resolve(typeof fn === "function" ? fn(args) : fn);
    const value =
      deadlineMs != null
        ? await Promise.race([
            run,
            new Promise((_, reject) =>
              setTimeout(
                () => reject(new Error(`tool call exceeded deadline (${deadlineMs}ms)`)),
                deadlineMs,
              ),
            ),
          ])
        : await run;
    emitLine({
      v: WIRE_VERSION,
      type: "result",
      req_id: reqId,
      ok: true,
      value,
    });
  } catch (err) {
    const kind =
      err instanceof Error && err.message.includes("deadline") ? "timeout" : "internal";
    emitLine({
      v: WIRE_VERSION,
      type: "result",
      req_id: reqId,
      ok: false,
      error: {
        kind,
        message: err instanceof Error ? err.message : String(err),
        traceback: err instanceof Error ? err.stack || "" : "",
      },
    });
  }
}

/** @type {Map<string, AbortController>} */
const inflight = new Map();

async function main() {
  const manifestPath = process.argv[2];
  if (manifestPath) {
    await loadManifest(manifestPath);
  }

  const rl = createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    let msg;
    try {
      msg = JSON.parse(trimmed);
    } catch {
      return;
    }

    const type = msg.type;
    const reqId = msg.req_id;

    if (type === "call" && reqId != null) {
      handleCall(msg).finally(() => inflight.delete(reqId));
      inflight.set(reqId, { abort: () => {} });
    } else if (type === "cancel" && reqId != null) {
      inflight.delete(reqId);
    } else if (type === "host_result") {
      deliverHostResult(msg);
    }
  });
}

main();
