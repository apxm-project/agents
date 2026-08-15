// Executes bundled TypeScript Capability handlers for packaging conformance.

import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { createInterface } from "node:readline";

import { isFunctionTool, isToolAnswer, makeHandlerId } from "./index.mjs";

const MANIFEST_VERSION = "apxm.handler-manifest";
const HANDLER_ID_PATTERN = /^sha256:[0-9a-f]{64}$/u;

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`handler manifest ${label} must be a non-empty string`);
  }
  return value;
}

function validateManifest(value) {
  if (!isRecord(value) || value.version !== MANIFEST_VERSION) {
    throw new Error(`handler manifest version must be ${MANIFEST_VERSION}`);
  }
  if (!Array.isArray(value.handlers)) {
    throw new Error("handler manifest handlers must be an array");
  }

  const seen = new Set();
  for (const [index, entry] of value.handlers.entries()) {
    if (!isRecord(entry) || entry.kind !== "tool" || entry.language !== "typescript") {
      throw new Error(`handler manifest entry ${index} is not a TypeScript Tool`);
    }
    const module = requireString(entry.module, `entry ${index} module`);
    const qualname = requireString(entry.qualname, `entry ${index} qualname`);
    const name = requireString(entry.name, `entry ${index} name`);
    const handlerId = requireString(entry.handler_id, `entry ${index} handler_id`);
    if (!HANDLER_ID_PATTERN.test(handlerId)) {
      throw new Error(`handler manifest entry ${index} has an invalid handler_id`);
    }
    if (handlerId !== makeHandlerId(module, qualname)) {
      throw new Error(
        `handler manifest entry ${index} handler_id does not match ${module}:${qualname}`,
      );
    }
    if (seen.has(handlerId)) {
      throw new Error(`handler manifest contains duplicate handler_id ${handlerId}`);
    }
    seen.add(handlerId);

    if (!isRecord(entry.source)) {
      throw new Error(`handler manifest entry ${index} source must be an object`);
    }
    const expectedArtifact = `handlers/${handlerId.slice("sha256:".length)}.mjs`;
    if (entry.source.artifact_path !== expectedArtifact) {
      throw new Error(
        `handler manifest entry ${index} artifact_path must be ${expectedArtifact}`,
      );
    }
    if (typeof entry.source.content !== "string" || entry.source.content.length === 0) {
      throw new Error(`handler manifest entry ${index} source content must be non-empty`);
    }
    if (!isRecord(entry.schema)) {
      throw new Error(`handler manifest entry ${index} schema must be an object`);
    }
    if (entry.read_only !== undefined && typeof entry.read_only !== "boolean") {
      throw new Error(`handler manifest entry ${index} read_only must be boolean`);
    }
    if (
      entry.requires_approval !== undefined &&
      typeof entry.requires_approval !== "boolean"
    ) {
      throw new Error(`handler manifest entry ${index} requires_approval must be boolean`);
    }

  }
  return value;
}

async function loadHandlers(manifestPath) {
  const manifest = validateManifest(JSON.parse(await readFile(manifestPath, "utf8")));
  const directory = await mkdtemp(path.join(tmpdir(), "apxm-handler-worker-"));
  const handlers = new Map();
  for (const entry of manifest.handlers ?? []) {
    const sourcePath = path.join(directory, `${entry.handler_id.slice("sha256:".length)}.mjs`);
    await writeFile(sourcePath, entry.source.content, "utf8");
    process.env.APXM_HANDLER_MODULE = entry.module;
    const exports = await import(pathToFileURL(sourcePath).href);
    const named = exports[entry.qualname];
    let handler;
    if (named !== undefined) {
      if (!isFunctionTool(named) || named.name !== entry.name) {
        throw new Error(
          `bundled handler ${entry.handler_id} does not export ${entry.qualname} as ${entry.name}`,
        );
      }
      handler = named;
    } else {
      const candidates = Object.values(exports).filter(
        (value) => isFunctionTool(value) && value.name === entry.name,
      );
      if (candidates.length !== 1) {
        throw new Error(`bundled handler ${entry.handler_id} is missing or ambiguous`);
      }
      handler = candidates[0];
    }
    handlers.set(entry.handler_id, handler.fn);
  }
  return { directory, handlers };
}

const manifestPath = process.argv[2];
if (!manifestPath) throw new Error("handler manifest path is required");
const { directory, handlers } = await loadHandlers(manifestPath);
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
try {
  for await (const line of lines) {
    if (!line.trim()) continue;
    const frame = JSON.parse(line);
    const handler = handlers.get(frame.tool_id);
    if (!handler) {
      console.log(
        JSON.stringify({
          v: 1,
          type: "result",
          req_id: frame.req_id,
          ok: false,
          error: `unknown handler ${frame.tool_id}`,
        }),
      );
      continue;
    }
    try {
      const answer = await handler(frame.args ?? {});
      if (!isToolAnswer(answer)) {
        throw new TypeError("a packaged Tool handler must return Tool.answer({...})");
      }
      console.log(
        JSON.stringify({ v: 1, type: "result", req_id: frame.req_id, ok: true, value: answer.value }),
      );
    } catch (error) {
      console.log(
        JSON.stringify({
          v: 1,
          type: "result",
          req_id: frame.req_id,
          ok: false,
          error: error instanceof Error ? error.message : String(error),
        }),
      );
    }
  }
} finally {
  await rm(directory, { recursive: true, force: true });
}
