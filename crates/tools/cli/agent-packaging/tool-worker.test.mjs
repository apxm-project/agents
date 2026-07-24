// Verifies the private packaging worker accepts only Tool.answer results.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
const packageDirectory = path.dirname(fileURLToPath(import.meta.url));
const workerPath = path.join(packageDirectory, "tool-worker.mjs");
const authoringModule = pathToFileURL(path.join(packageDirectory, "index.mjs")).href;

test("the worker rejects a handler result that bypasses Tool.answer", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "apxm-tool-worker-test-"));
  const manifestPath = path.join(directory, "handlers.json");
  const handlerId = "sha256:plain-result-fixture";
  const source = [
    `import { Tool } from ${JSON.stringify(authoringModule)};`,
    "export const plainResult = Tool.define({",
    '  name: "plain_result",',
    '  description: "Invalid private worker fixture.",',
    "  input: Tool.object({ message: Tool.text({ minLength: 1 }) }),",
    "  run({ message }) { return { message }; },",
    "});",
  ].join("\n");
  const manifest = {
    version: "apxm.handler-manifest.v1",
    handlers: [{
      kind: "tool",
      handler_id: handlerId,
      module: "fixtures/plain-result",
      qualname: "plainResult",
      name: "plain_result",
      source: { artifact_path: "handlers/plain-result.mjs", content: source },
    }],
  };

  try {
    await writeFile(manifestPath, JSON.stringify(manifest));
    const child = spawn(process.execPath, [workerPath, manifestPath], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    child.stdin.end(`${JSON.stringify({
      v: 1,
      type: "call",
      req_id: "plain-result",
      tool_id: handlerId,
      args: { message: "hello" },
    })}\n`);

    const [stdout, stderr] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ]);
    const exitCode = await new Promise((resolve) => child.once("close", resolve));
    assert.equal(exitCode, 0, stderr);
    assert.deepEqual(JSON.parse(stdout), {
      v: 1,
      type: "result",
      req_id: "plain-result",
      ok: false,
      error: "a packaged Tool handler must return Tool.answer({...})",
    });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
