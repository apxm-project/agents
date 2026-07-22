// Generic TypeScript AgentProgram records effects and structured control flow.

import { describe, expect, it } from "vitest";

import { AgentProgram, decodeFact } from "../src/index.ts";

describe("AgentProgram", () => {
  it("records generic effects and a structured loop", () => {
    const program = new AgentProgram({
      program_id: "generic",
      input_type_ref: "Input",
      output_type_ref: "Output",
      context_type_ref: "Context",
    });
    program.loop("region.loop", (body) => body.modelCall("node.model", "model.default"));
    program.capabilityInvoke("node.capability", "cap.search");
    program.awaitEvent("node.await", "event.input");
    program.returnRegion("region.return");

    const graph = program.buildGraph() as {
      semantic_operations: Array<{
        op: string;
        parent_region_id: string;
        execution_order: number;
      }>;
      structural_regions: Array<Record<string, unknown>>;
    };
    expect(graph.semantic_operations.map((operation) => operation.op)).toEqual([
      "model.call",
      "capability.invoke",
      "await.event",
    ]);
    expect(graph.semantic_operations[0]).toMatchObject({
      parent_region_id: "region.loop",
      execution_order: 0,
    });
    expect(graph.semantic_operations[1]).toMatchObject({
      parent_region_id: "region.generic.body",
      execution_order: 1,
    });
    expect(graph.structural_regions).toEqual([
      {
        region_id: "region.generic.body",
        kind: "region",
        execution_order: 0,
      },
      {
        region_id: "region.loop",
        kind: "ais.loop",
        parent_region_id: "region.generic.body",
        execution_order: 0,
      },
      {
        region_id: "region.return",
        kind: "return",
        parent_region_id: "region.generic.body",
        execution_order: 3,
      },
    ]);
    expect(
      (program.buildGraph() as { source_map: { region_annotations: unknown[] } })
        .source_map.region_annotations,
    ).toEqual([{ region_id: "region.loop", annotation: "structural_loop" }]);
  });

  it("exposes every author-facing structural kind", () => {
    const program = new AgentProgram({
      program_id: "structured",
      input_type_ref: "Input",
      output_type_ref: "Output",
    });
    const empty = () => undefined;

    program.branch("region.branch", empty, empty);
    program.switch("region.switch", [empty, empty]);
    program.loop("region.loop", empty);
    program.parallel("region.parallel", empty);
    program.tryCatch("region.try", "region.catch", empty, empty);
    program.throwRegion("region.throw");
    program.returnRegion("region.return");
    program.yieldRegion("region.yield");

    const graph = program.buildGraph() as {
      structural_regions: Array<Record<string, unknown>>;
    };
    expect(graph.structural_regions).toEqual([
      {
        region_id: "region.structured.body",
        kind: "region",
        execution_order: 0,
      },
      ...[
        ["region.branch", "branch"],
        ["region.switch", "switch"],
        ["region.loop", "ais.loop"],
        ["region.parallel", "parallel_join"],
        ["region.try", "try"],
        ["region.catch", "catch"],
        ["region.throw", "throw"],
        ["region.return", "return"],
        ["region.yield", "yield"],
      ].map(([region_id, kind], execution_order) => ({
        region_id,
        kind,
        parent_region_id: "region.structured.body",
        execution_order,
      })),
    ]);
    expect("recordRegion" in program).toBe(false);
  });

  it("records nested and sibling loop containment", () => {
    const program = new AgentProgram({
      program_id: "nested",
      input_type_ref: "Input",
      output_type_ref: "Output",
    });
    program.loop("loop.outer", (outer) => {
      outer.modelCall("node.outer.before", "model.default");
      outer.loop("loop.inner", (inner) =>
        inner.capabilityInvoke("node.inner", "cap.inner"),
      );
      outer.modelCall("node.outer.after", "model.default");
    });
    program.loop("loop.sibling", (sibling) =>
      sibling.capabilityInvoke("node.sibling", "cap.sibling"),
    );

    const graph = program.buildGraph() as {
      structural_regions: Array<{
        region_id: string;
        parent_region_id?: string;
        execution_order: number;
      }>;
      semantic_operations: Array<{
        node_id: string;
        parent_region_id: string;
        execution_order: number;
      }>;
      source_map: { region_annotations: Array<{ annotation: string }> };
    };
    const regions = new Map(
      graph.structural_regions.map((region) => [region.region_id, region]),
    );
    const nodes = new Map(
      graph.semantic_operations.map((operation) => [operation.node_id, operation]),
    );
    expect(regions.get("loop.outer")).toMatchObject({
      parent_region_id: "region.nested.body",
      execution_order: 0,
    });
    expect(regions.get("loop.inner")).toMatchObject({
      parent_region_id: "loop.outer",
      execution_order: 1,
    });
    expect(regions.get("loop.sibling")).toMatchObject({
      parent_region_id: "region.nested.body",
      execution_order: 1,
    });
    expect(nodes.get("node.outer.before")).toMatchObject({
      parent_region_id: "loop.outer",
      execution_order: 0,
    });
    expect(nodes.get("node.inner")).toMatchObject({
      parent_region_id: "loop.inner",
      execution_order: 0,
    });
    expect(nodes.get("node.outer.after")?.execution_order).toBe(2);
    expect(nodes.get("node.sibling")?.parent_region_id).toBe("loop.sibling");
    expect(
      new Set(
        graph.source_map.region_annotations.map(
          (annotation) => annotation.annotation,
        ),
      ),
    ).toEqual(new Set(["structural_loop"]));
  });

  it("publishes a closed generated runtime evidence decoder", () => {
    expect(
      decodeFact({
        fact_id: "loop.1",
        event_sequence: 1,
        fact_kind: "LoopIterationCompleted",
        static_loop_id: "loop.main",
        loop_occurrence_id: "occurrence.1",
        iteration_index: 0,
        program_invocation_id: "invocation.1",
        causal_node_execution_ids: ["node-execution.1"],
      }).fact_kind,
    ).toBe("LoopIterationCompleted");
    expect(() =>
      decodeFact({
        fact_id: "bad.1",
        event_sequence: 1,
        fact_kind: "invented.fact",
      }),
    ).toThrow(/unknown fact_kind/);
  });
});
