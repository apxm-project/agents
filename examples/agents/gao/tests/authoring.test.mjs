// Gao remains an example-local specialization over installed generic APIs.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { ConversationalAgent } from "@apxm/example-agent-conversational";
import { Gao, buildGao } from "../dist/index.js";

test("Gao extends the example-local ConversationalAgent", () => {
  assert.ok(new Gao() instanceof ConversationalAgent);
});

test("Gao records only generic composition and structural operations", () => {
  const program = buildGao();
  const graph = program.buildGraph();
  assert.deepEqual(
    graph.semantic_operations.map((operation) => operation.op),
    [
      "model.call",
      "capability.invoke",
      "program.new",
      "program.invoke",
      "await.event",
    ],
  );
  assert.deepEqual(
    graph.structural_regions.map((region) => region.kind),
    ["region", "ais.loop", "return"],
  );
  assert.match(JSON.stringify(program.lowerGraph()), /"ais\.loop"/);
});

test("Gao source has no compiler-private import or direct AIR emitter", () => {
  const source = readFileSync(new URL("../src/gao.ts", import.meta.url), "utf8");
  assert.equal(source.includes("crates/compiler"), false);
  assert.equal(source.includes("GraphBuilder"), false);
  assert.equal(source.includes("emit_air"), false);
});

test("Gao source spans point to authored composition calls", () => {
  const source = readFileSync(new URL("../src/gao.ts", import.meta.url), "utf8");
  const lines = source.split(/\r?\n/);
  const spans = new Map(
    buildGao().buildGraph().source_map.node_spans.map((span) => [span.node_id, span]),
  );
  for (const [nodeId, authoredCall] of [
    ["node.specialist.new", '.programNew("node.specialist.new"'],
    ["node.specialist.invoke", '.programInvoke("node.specialist.invoke"'],
  ]) {
    const span = spans.get(nodeId).span;
    const line = lines[span.start_line - 1];
    assert.equal(line.slice(span.start_column, span.end_column), authoredCall);
  }
});
