// Holds the handler manifest this package produces against the published
// `apxm.handler-manifest` schema it names. A producer emitting a manifest
// the contract rejects fails here, at production, not at a downstream consumer.

import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { compileHandlers } from "./compile-handlers.mjs";
import { makeHandlerId } from "./index.mjs";

const packageDirectory = path.dirname(fileURLToPath(import.meta.url));
const agentsRoot = path.resolve(packageDirectory, "../../../..");
const workspaceRoot = path.resolve(agentsRoot, "..");

const schema = JSON.parse(
  readFileSync(
    path.join(agentsRoot, "contracts/schemas/apxm.handler-manifest.json"),
    "utf8",
  ),
);
// `apxm.contract-common.v1` is owned by the coordinating APXM workspace, not by
// this repository, so it is only present in a full-workspace checkout. A
// standalone `agents` clone (and CI, which checks out `workspace/agents` alone)
// cannot resolve it. Skip rather than fail: an unreadable foreign contract is a
// missing input, not a manifest violation.
const commonPath = path.join(workspaceRoot, "contracts/schemas/contract-common.v1.json");
let common = null;
let skip = false;
try {
  common = JSON.parse(readFileSync(commonPath, "utf8"));
} catch {
  skip = `shared workspace contract not present at ${commonPath}`;
}

// The subset of JSON Schema the published handler-manifest contract uses. Each
// keyword below appears in that schema; an unrecognized keyword is a hard error
// rather than a silent pass, so the gate cannot weaken as the schema grows.
const SUPPORTED = new Set([
  "$schema", "$id", "$ref", "$defs", "title", "description", "oneOf",
  "type", "const", "enum", "required", "properties", "additionalProperties",
  "items", "pattern", "minLength",
]);

function resolveRef(ref, root) {
  if (ref.startsWith("#/$defs/")) return [root.$defs[ref.slice("#/$defs/".length)], root];
  const [id, pointer] = ref.split("#/$defs/");
  const external = id === common.$id ? common : null;
  assert.ok(external, `unresolved external schema ${id}`);
  return pointer ? [external.$defs[pointer], external] : [external, external];
}

function typeOk(instance, name) {
  if (name === "object") return instance !== null && typeof instance === "object" && !Array.isArray(instance);
  if (name === "array") return Array.isArray(instance);
  if (name === "string") return typeof instance === "string";
  if (name === "boolean") return typeof instance === "boolean";
  if (name === "integer") return Number.isInteger(instance);
  if (name === "number") return typeof instance === "number";
  if (name === "null") return instance === null;
  throw new Error(`unsupported schema type ${name}`);
}

/** Collect every way `instance` violates `node`; an empty list means valid. */
function violations(node, instance, root) {
  for (const keyword of Object.keys(node)) {
    assert.ok(SUPPORTED.has(keyword), `unsupported schema keyword ${keyword}`);
  }
  if (node.$ref) {
    const [target, targetRoot] = resolveRef(node.$ref, root);
    return violations(target, instance, targetRoot);
  }
  if (node.oneOf) {
    const matched = node.oneOf.filter((branch) => violations(branch, instance, root).length === 0);
    return matched.length === 1 ? [] : [`matched ${matched.length} oneOf branches, expected 1`];
  }

  const errors = [];
  if ("const" in node && instance !== node.const) {
    return [`expected const ${JSON.stringify(node.const)}`];
  }
  if (node.enum && !node.enum.includes(instance)) {
    errors.push(`${JSON.stringify(instance)} is not one of ${JSON.stringify(node.enum)}`);
  }
  if (node.type && !typeOk(instance, node.type)) {
    return [`expected type ${node.type}`];
  }
  if (typeof instance === "string") {
    if (node.minLength !== undefined && instance.length < node.minLength) {
      errors.push("string is shorter than minLength");
    }
    if (node.pattern !== undefined && !new RegExp(node.pattern, "u").test(instance)) {
      errors.push(`string does not match pattern ${node.pattern}`);
    }
  }
  if (Array.isArray(instance) && node.items) {
    instance.forEach((item, index) => {
      for (const error of violations(node.items, item, root)) errors.push(`[${index}] ${error}`);
    });
  }
  if (instance !== null && typeof instance === "object" && !Array.isArray(instance)) {
    for (const key of node.required ?? []) {
      if (!(key in instance)) errors.push(`'${key}' is required`);
    }
    for (const [key, value] of Object.entries(instance)) {
      const property = node.properties?.[key];
      if (property) {
        for (const error of violations(property, value, root)) errors.push(`${key}: ${error}`);
      } else if (node.additionalProperties === false) {
        errors.push(`additional property '${key}' is not allowed`);
      }
    }
  }
  return errors;
}

const assertValid = (manifest) =>
  assert.deepEqual(violations(schema, manifest, schema), [], "manifest violates its own contract");

async function compileFixture(source) {
  const root = await mkdtemp(path.join(tmpdir(), "apxm-compile-handlers-test-"));
  try {
    const entry = path.join(root, "capabilities", "echo", "handler.mjs");
    await mkdir(path.dirname(entry), { recursive: true });
    await writeFile(entry, source, "utf8");
    return await compileHandlers([entry], { rootDir: root });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("a compiled handler manifest satisfies the published schema", { skip }, async () => {
  const manifest = await compileFixture(
    [
      'import { Tool } from "@apxm/agent-packaging";',
      "export const echo = Tool.define({",
      '  name: "echo",',
      '  description: "Return an input message without executing a runtime effect.",',
      "  input: Tool.object({ message: Tool.text({ minLength: 1 }) }),",
      "  run({ message }) { return Tool.answer({ message }); },",
      "});",
    ].join("\n"),
  );

  assert.equal(manifest.version, schema.properties.version.const);
  assert.equal(manifest.handlers.length, 1);
  const [handler] = manifest.handlers;
  assert.equal(handler.kind, "tool");
  assert.equal(handler.language, "typescript");
  // The producer content-addresses the handler by module and qualname; recompute
  // that identity from the same inputs rather than pinning a literal digest.
  assert.equal(handler.handler_id, makeHandlerId(handler.module, handler.qualname));
  assert.equal(
    handler.source.artifact_path,
    `handlers/${handler.handler_id.slice("sha256:".length)}.mjs`,
  );
  assertValid(manifest);
});

test("the gate rejects a manifest the published schema rejects", { skip }, () => {
  // A build-host absolute path is exactly what `HandlerSource.artifact_path`
  // forbids; if this passed, the gate above would prove nothing.
  const handlerId = makeHandlerId("capabilities/echo/handler", "echo");
  assert.notDeepEqual(
    violations(
      schema,
      {
        version: "apxm.handler-manifest",
        handlers: [{
          kind: "tool",
          language: "typescript",
          handler_id: handlerId,
          module: "capabilities/echo/handler",
          qualname: "echo",
          name: "echo",
          source: { artifact_path: "/build/echo.mjs", content: "export function echo() {}\n" },
          schema: { type: "object" },
        }],
      },
      schema,
    ),
    [],
    "a build-host artifact path must not satisfy the contract",
  );
});
