// Executes bundled TypeScript Capability handlers for packaging conformance.

import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { createInterface } from "node:readline";

import { isFunctionTool, isToolAnswer } from "./index.mjs";

async function loadHandlers(manifestPath) {
  const manifest = JSON.parse(await readFile(manifestPath, "utf8"));
  const directory = await mkdtemp(path.join(tmpdir(), "apxm-handler-worker-"));
  const handlers = new Map();
  for (const entry of manifest.handlers ?? []) {
    const sourcePath = path.join(directory, `${entry.handler_id.slice("sha256:".length)}.mjs`);
    await writeFile(sourcePath, entry.source.content, "utf8");
    process.env.APXM_HANDLER_MODULE = entry.module;
    const exports = await import(pathToFileURL(sourcePath).href);
    const named = exports[entry.qualname];
    const handler = isFunctionTool(named)
      ? named
      : Object.values(exports).find(
          (value) => isFunctionTool(value) && value.name === entry.name,
        );
    if (!handler) throw new Error(`bundled handler ${entry.handler_id} is missing`);
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
