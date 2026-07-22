// TypeScript canonical FrontendGraph and AIR parity fixtures.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { canonicalAirJson, lower, verify } from "../../native/typescript/js/index.ts";
import {
  externalAgentGraph,
  gaoConversationalGraph,
  specialistGraph,
} from "../../native/typescript/js/example.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PARITY_DIR = join(__dirname, "../../native/parity");
const FIVE_OPS = new Set([
  "model.call",
  "capability.invoke",
  "program.new",
  "program.invoke",
  "await.event",
]);

function readFixture(name: string): unknown {
  return JSON.parse(readFileSync(join(PARITY_DIR, name), "utf8")) as unknown;
}

function readGolden(name: string): string {
  return readFileSync(join(PARITY_DIR, name), "utf8").trim();
}

function normalizeFrontendSource(air: Record<string, unknown>): Record<string, unknown> {
  const value = structuredClone(air) as {
    source_map: {
      source_language: string;
      node_spans: Array<{ source_file: string }>;
    };
  };
  value.source_map.source_language = "frontend";
  value.source_map.node_spans.forEach((span) => {
    span.source_file = "frontend";
  });
  return value as unknown as Record<string, unknown>;
}

describe("canonical TypeScript FrontendGraph parity", () => {
  it("matches the canonical shared FrontendGraph fixture", () => {
    expect(specialistGraph()).toEqual(readFixture("frontend-graph.example.json"));
  });

  it("lowers to the canonical AIR fixture", () => {
    expect(canonicalAirJson(specialistGraph())).toBe(readGolden("air.expected.json"));
  });

  it("lowers deterministically", () => {
    const graph = specialistGraph();
    expect(canonicalAirJson(graph)).toBe(canonicalAirJson(graph));
  });

  it("rejects retired operation names at the canonical graph boundary", () => {
    const graph = specialistGraph() as Record<string, unknown>;
    (graph.semantic_operations as unknown[]).push({ node_id: "node.bad", op: "ASK" });
    expect(verify(graph)).not.toBeNull();
  });

  it("lowers an external agent to one capability.invoke", () => {
    const air = lower(externalAgentGraph()) as { semantic_operations: { op: string }[] };
    expect(air.semantic_operations.map((op) => op.op)).toEqual(["capability.invoke"]);
  });

  it("matches the external-agent canonical AIR fixture", () => {
    expect(canonicalAirJson(externalAgentGraph())).toBe(
      readGolden("air.external-agent.expected.json"),
    );
  });

  it("keeps Gao on the five canonical semantic operations", () => {
    const air = lower(gaoConversationalGraph()) as { semantic_operations: { op: string }[] };
    for (const op of air.semantic_operations) {
      expect(FIVE_OPS.has(op.op), op.op).toBe(true);
    }
  });

  it("matches Gao semantics while retaining TypeScript source identity", () => {
    const actual = lower(gaoConversationalGraph()) as Record<string, unknown>;
    const sourceMap = actual.source_map as {
      source_language: string;
      node_spans: Array<{ source_file: string }>;
    };
    expect(sourceMap.source_language).toBe("typescript");
    expect(sourceMap.node_spans.every((span) => span.source_file.endsWith(".ts"))).toBe(true);
    expect(normalizeFrontendSource(actual)).toEqual(
      normalizeFrontendSource(JSON.parse(readGolden("air.gao.expected.json"))),
    );
  });
});
