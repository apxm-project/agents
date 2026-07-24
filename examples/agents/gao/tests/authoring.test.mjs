// Gao is an ordinary Agent authored on the installed typed frontend.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { buildGao } from "../dist/index.js";

test("Gao verifies and lowers to only generic operations", () => {
  const gao = buildGao();
  assert.equal(gao.diagnostics(), null);
  const air = JSON.parse(gao.canonicalAir());
  const ops = new Set(air.semantic_operations.map((operation) => operation.op));
  assert.ok(ops.has("model.call"));
  assert.ok(ops.has("capability.invoke"));
  assert.ok(ops.has("await.event"));
  const structural = air.structural_ir.map((node) => node.kind);
  assert.ok(structural.includes("ais.loop"));
});

test("Gao source uses only the public authoring surface", () => {
  const source = readFileSync(new URL("../src/gao.ts", import.meta.url), "utf8");
  assert.equal(source.includes("crates/compiler"), false);
  assert.equal(source.includes("AgentProgram"), false);
  assert.equal(source.includes("sourceSpan"), false);
  assert.equal(source.includes("programNew"), false);
});
