// ConversationalAgent and Gao parity with the shared golden graphs.

import { test } from "node:test";
import assert from "node:assert/strict";

import { canonicalAirJson, compileArtifact, lower, verify } from "./index.ts";
import { gaoConversationalGraph } from "./gao.ts";
import { Gao } from "./gao.ts";

test("Gao builds through ConversationalAgent without GraphBuilder escape", () => {
  const graph = Gao.build().buildGraph();
  assert.strictEqual(verify(graph), null);
  assert.deepStrictEqual(graph, gaoConversationalGraph());
});

test("Gao lowers to the shared golden AIR", () => {
  const graph = Gao.build().buildGraph();
  assert.strictEqual(canonicalAirJson(graph), canonicalAirJson(gaoConversationalGraph()));
});

test("public Gao authoring builds a complete executable artifact", () => {
  const graph = Gao.build().buildGraph();
  const artifact = compileArtifact(graph);

  assert.equal(artifact.schema_version, "apxm.executable-artifact.v1");
  assert.deepStrictEqual(artifact.air, lower(graph));
  const entrypoints = artifact.entrypoints as Array<Record<string, unknown>>;
  assert.equal(entrypoints[0]?.program_id, "Gao");
});
