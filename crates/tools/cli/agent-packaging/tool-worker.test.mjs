// Verifies the private packaging worker accepts only Tool.answer results.

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { makeHandlerId } from "./index.mjs";

const packageDirectory = path.dirname(fileURLToPath(import.meta.url));
const workerPath = path.join(packageDirectory, "tool-worker.mjs");
const authoringModule = pathToFileURL(path.join(packageDirectory, "index.mjs")).href;
const nodeExecutable =
  process.env.APXM_NODE_BIN ??
  process.env.PATH.split(path.delimiter)
    .map((directory) => path.join(directory, "node"))
    .find((candidate) => existsSync(candidate)) ??
  process.argv0;

function descriptor({ module, qualname, name, content }) {
  const handlerId = makeHandlerId(module, qualname);
  return {
    kind: "tool",
    language: "typescript",
    handler_id: handlerId,
    module,
    qualname,
    name,
    source: {
      artifact_path: `handlers/${handlerId.slice("sha256:".length)}.mjs`,
      content,
    },
    schema: {
      type: "object",
      properties: { message: { type: "string", minLength: 1 } },
      required: ["message"],
      additionalProperties: false,
    },
    read_only: true,
    requires_approval: false,
  };
}

test("the worker rejects a handler result that bypasses Tool.answer", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "apxm-tool-worker-test-"));
  const manifestPath = path.join(directory, "handlers.json");
  const module = "fixtures/plain-result";
  const qualname = "plainResult";
  const source = [
    `import { Tool } from ${JSON.stringify(authoringModule)};`,
    "export const plainResult = Tool.define({",
    '  name: "plain_result",',
    '  description: "Invalid private worker fixture.",',
    "  readOnly: true,",
    "  input: Tool.object({ additionalProperties: false, properties: { message: Tool.text({ required: true, minLength: 1 }) } }),",
    "  run({ message }) { return { message }; },",
    "});",
  ].join("\n");
  const handlerId = makeHandlerId(module, qualname);
  const manifest = {
    version: "apxm.handler-manifest",
    handlers: [descriptor({ module, qualname, name: "plain_result", content: source })],
  };

  try {
    await writeFile(manifestPath, JSON.stringify(manifest));
    const child = spawn(nodeExecutable, [workerPath, manifestPath], {
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

test("the worker unwraps a typed Tool.answer only at its private boundary", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "apxm-tool-worker-test-"));
  const manifestPath = path.join(directory, "handlers.json");
  const module = "fixtures/typed-answer";
  const qualname = "typedAnswer";
  const source = [
    `import { Tool } from ${JSON.stringify(authoringModule)};`,
    "export const typedAnswer = Tool.define({",
    '  name: "typed_answer",',
    '  description: "Private worker fixture.",',
    "  readOnly: true,",
    "  input: Tool.object({ additionalProperties: false, properties: { message: Tool.text({ required: true, minLength: 1 }) } }),",
    "  run({ message }) { return Tool.answer({ message }); },",
    "});",
  ].join("\n");
  const handlerId = makeHandlerId(module, qualname);
  const manifest = {
    version: "apxm.handler-manifest",
    handlers: [descriptor({ module, qualname, name: "typed_answer", content: source })],
  };

  try {
    await writeFile(manifestPath, JSON.stringify(manifest));
    const child = spawn(nodeExecutable, [workerPath, manifestPath], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    child.stdin.end(`${JSON.stringify({
      v: 1,
      type: "call",
      req_id: "typed-answer",
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
      req_id: "typed-answer",
      ok: true,
      value: { message: "hello" },
    });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("the worker refuses a descriptor whose materialized identity is unsafe", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "apxm-tool-worker-test-"));
  const manifestPath = path.join(directory, "handlers.json");
  const module = "fixtures/unsafe";
  const qualname = "unsafe";
  const handlerId = makeHandlerId(module, qualname);
  const manifest = {
    version: "apxm.handler-manifest",
    handlers: [{
      ...descriptor({
        module,
        qualname,
        name: "unsafe",
        content: "export const unsafe = {};\n",
      }),
      source: {
        artifact_path: "handlers/../../outside.mjs",
        content: "export const unsafe = {};\n",
      },
    }],
  };

  try {
    assert.equal(manifest.handlers[0].handler_id, handlerId);
    await writeFile(manifestPath, JSON.stringify(manifest));
    const child = spawn(nodeExecutable, [workerPath, manifestPath], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    child.stdin.end();
    const [stdout, stderr] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
    ]);
    const exitCode = await new Promise((resolve) => child.once("close", resolve));
    assert.notEqual(exitCode, 0, stdout);
    assert.match(stderr, /artifact_path must be handlers\//);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
