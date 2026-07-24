// Source-first TypeScript authoring produces the typed FrontendGraph.

import { describe, expect, it } from "vitest";

import { Agent, Context, Model, Tool, decodeFact } from "../src/index.ts";

const SummarizerModel = Model("summarizer.model.v1");

const Summarizer = Agent({
  name: "Summarizer",
  use: { SummarizerModel },
  async run(agent, request) {
    return await SummarizerModel(request);
  },
});

const ConversationCtx = Context({ messages: [] as string[] });
const SearchWeb = Tool("search.web.capability.v1");
const SupportModel = Model("support.model.v1");

const Support = Agent({
  name: "Support",
  context: ConversationCtx,
  use: { SearchWeb, SupportModel },
  async run(agent, incoming) {
    while (true) {
      let research = null;
      if (incoming !== null) {
        research = await SearchWeb(incoming);
      }
      const response = await SupportModel(incoming);
      agent.context = { messages: [] };
      return response;
    }
  },
});

type Graph = {
  schema_version: string;
  declarations: Array<{ decl_kind: string }>;
  call_intents: Array<{ intent_kind: string }>;
  control_intents: Array<{ control_kind: string }>;
  capability_requirements: Array<{ capability_ref: string; tool_schema_present: boolean }>;
};

describe("source-first TypeScript authoring", () => {
  it("binds a minimal one-shot Model agent", () => {
    const graph = Summarizer.frontendGraph() as unknown as Graph;
    expect(graph.schema_version).toBe("apxm.frontend-graph.v1");
    expect(graph.declarations.map((d) => d.decl_kind)).toEqual(["model_binding"]);
    expect(graph.call_intents.map((c) => c.intent_kind)).toEqual(["model_invocation"]);
    expect(Summarizer.diagnostics()).toBeNull();
  });

  it("lowers the minimal agent to a registered model.call", () => {
    const air = JSON.parse(Summarizer.canonicalAir()) as {
      semantic_operations: Array<{ op: string; operands: Array<{ slot: string }> }>;
    };
    expect(air.semantic_operations.map((o) => o.op)).toEqual(["model.call"]);
    expect(air.semantic_operations[0].operands.some((o) => o.slot === "request")).toBe(true);
  });

  it("binds a contextual Tool-using loop agent", () => {
    const graph = Support.frontendGraph() as unknown as Graph;
    expect(new Set(graph.declarations.map((d) => d.decl_kind))).toEqual(
      new Set(["context", "tool_binding", "model_binding"]),
    );
    expect(graph.call_intents.map((c) => c.intent_kind)).toEqual([
      "tool_invocation",
      "model_invocation",
    ]);
    expect(new Set(graph.control_intents.map((c) => c.control_kind))).toEqual(
      new Set(["loop", "conditional", "return"]),
    );
    expect(graph.capability_requirements).toEqual([
      { capability_ref: "search.web.capability.v1", tool_schema_present: true },
    ]);
    expect(Support.diagnostics()).toBeNull();
  });

  it("lowers a Tool call to capability.invoke inside ais.loop", () => {
    const air = JSON.parse(Support.canonicalAir()) as {
      semantic_operations: Array<{ op: string }>;
      structural_ir: Array<{ kind: string }>;
    };
    const ops = new Set(air.semantic_operations.map((o) => o.op));
    expect(ops.has("capability.invoke")).toBe(true);
    expect(ops.has("model.call")).toBe(true);
    const structural = air.structural_ir.map((n) => n.kind);
    expect(structural).toContain("ais.loop");
    expect(structural).toContain("branch");
  });

  it("keeps runtime evidence decoding closed", () => {
    const fact = decodeFact({
      fact_id: "loop.1",
      event_sequence: 1,
      fact_kind: "LoopIterationCompleted",
      static_loop_id: "loop.main",
      loop_occurrence_id: "occurrence.1",
      iteration_index: 0,
      program_invocation_id: "invocation.1",
      causal_node_execution_ids: ["node-execution.1"],
    });
    expect(fact.fact_kind).toBe("LoopIterationCompleted");
    expect(() =>
      decodeFact({ fact_id: "bad", event_sequence: 1, fact_kind: "invented" }),
    ).toThrow();
  });
});
