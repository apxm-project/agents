// TypeScript authoring frontend: record a FrontendGraph and lower it to
// canonical AIR through the in-process Node-API bridge (no CLI, no network).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { canonicalAirJson, lower, verify } from "./index.ts";
import { specialistGraph } from "./example.ts";

const here = dirname(fileURLToPath(import.meta.url));
const parityDir = join(here, "..", "..", "parity");
const goldenGraph = JSON.parse(readFileSync(join(parityDir, "frontend-graph.example.json"), "utf8"));
const goldenAir = readFileSync(join(parityDir, "air.expected.json"), "utf8").trim();

test("authored graph matches golden input", () => {
  assert.deepStrictEqual(specialistGraph(), goldenGraph);
});

test("lower matches golden AIR", () => {
  assert.strictEqual(canonicalAirJson(specialistGraph()), goldenAir);
});

test("lowering is deterministic", () => {
  const graph = specialistGraph();
  assert.strictEqual(canonicalAirJson(graph), canonicalAirJson(graph));
});

test("verify accepts a valid graph", () => {
  assert.strictEqual(verify(specialistGraph()), null);
});

test("verify rejects an unknown operation", () => {
  const graph = specialistGraph() as Record<string, unknown>;
  (graph.semantic_operations as unknown[]).push({ node_id: "node.bad", op: "tool.loop" });
  assert.notStrictEqual(verify(graph), null);
});

test("lower rejects an unknown operation", () => {
  const graph = specialistGraph() as Record<string, unknown>;
  (graph.semantic_operations as unknown[]).push({ node_id: "node.bad", op: "tool.loop" });
  assert.throws(() => lower(graph));
});
