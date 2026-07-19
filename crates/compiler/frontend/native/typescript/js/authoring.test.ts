// TypeScript authoring frontend: record a FrontendGraph and lower it to
// canonical AIR through the in-process Node-API bridge (no CLI, no network).

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { canonicalAirJson, lower, verify } from "./index.ts";
import { externalAgentGraph, gaoConversationalGraph, specialistGraph } from "./example.ts";

const here = dirname(fileURLToPath(import.meta.url));
const parityDir = join(here, "..", "..", "parity");
const goldenGraph = JSON.parse(readFileSync(join(parityDir, "frontend-graph.example.json"), "utf8"));
const goldenAir = readFileSync(join(parityDir, "air.expected.json"), "utf8").trim();
const goldenAcpAir = readFileSync(join(parityDir, "air.external-agent.expected.json"), "utf8").trim();
const goldenGaoAir = readFileSync(join(parityDir, "air.gao.expected.json"), "utf8").trim();
const FIVE_OPS = new Set([
  "model.call",
  "capability.invoke",
  "program.new",
  "program.invoke",
  "await.event",
]);

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

test("external agent capability lowers only to capability.invoke", () => {
  const air = lower(externalAgentGraph()) as { semantic_operations: { op: string }[] };
  const ops = air.semantic_operations.map((o) => o.op);
  assert.deepStrictEqual(ops, ["capability.invoke"]);
});

test("external agent AIR matches golden", () => {
  assert.strictEqual(canonicalAirJson(externalAgentGraph()), goldenAcpAir);
});

test("gao conversational agent lowers to only the five semantic ops", () => {
  const air = lower(gaoConversationalGraph()) as { semantic_operations: { op: string }[] };
  for (const op of air.semantic_operations) {
    assert.ok(FIVE_OPS.has(op.op), `unexpected op ${op.op}`);
  }
});

test("gao AIR matches golden", () => {
  assert.strictEqual(canonicalAirJson(gaoConversationalGraph()), goldenGaoAir);
});

test("lower rejects an unknown operation", () => {
  const graph = specialistGraph() as Record<string, unknown>;
  (graph.semantic_operations as unknown[]).push({ node_id: "node.bad", op: "tool.loop" });
  assert.throws(() => lower(graph));
});
