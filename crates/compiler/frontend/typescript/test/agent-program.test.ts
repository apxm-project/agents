// Source-first TypeScript authoring produces the typed FrontendGraph.

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import "../src/node.ts";
import { Agent, Capability, Context, Hook, Model, TaskGroup, Tool } from "../src/index.ts";
import { CaptureError, captureProgram } from "../src/capture.ts";
import { decodeFact } from "../src/generated/runtime-evidence.ts";

const sourceFile = fileURLToPath(import.meta.url);
const source = { fileName: sourceFile, text: readFileSync(sourceFile, "utf8") };

const SummarizerModel = Model("summarizer.model.v1");

const Summarizer = Agent({
  name: "Summarizer",
  source,
  use: { SummarizerModel },
  async run(agent, request) {
    return await SummarizerModel(request);
  },
});

const ConversationCtx = Context({ messages: [] as string[] });
const SearchWeb = Tool("search.web.capability.v1");
const SupportModel = Model("support.model.v1");

const Specialist = Agent({
  name: "Specialist",
  source,
  use: { SearchWeb },
  async run(_agent, request) {
    return await SearchWeb(request);
  },
});

const Support = Agent({
  name: "Support",
  source,
  context: ConversationCtx,
  use: { SearchWeb, Specialist, SupportModel },
  async run(agent, incoming) {
    while (true) {
      let research = null;
      if (incoming !== null) {
        await TaskGroup.run(async () => {
          research = await SearchWeb(incoming);
        });
      }
      const specialist = Specialist.new({ context: { messages: [] } });
      const review = await specialist.invoke(incoming);
      const response = await SupportModel({ incoming, research, review });
      agent.context = { messages: [] };
      incoming = await agent.yield_(response);
    }
  },
});

const SupportPolicy = Hook.before({
  agent: Support,
  target: SupportModel,
  scope: "model",
  async run(agent) {
    void agent.context;
  },
});
void SupportPolicy;

type Graph = {
  schema_version: string;
  declarations: Array<{ decl_kind: string }>;
  values: Array<{
    value_id: string;
    type_ref: string;
    origin: string;
    origin_id?: string;
  }>;
  blocks: Array<{
    block_id: string;
    region_id: string;
    block_arguments: string[];
    execution_order: number;
  }>;
  regions: Array<{ region_id: string }>;
  call_intents: Array<{ intent_kind: string }>;
  control_intents: Array<{ control_kind: string }>;
  context_flow: Array<{ from_node: string; to_node: string }>;
  hook_bindings: Array<{ scope: string; phase: string }>;
  imported_program_refs: Array<{ program_ref: string }>;
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
      "agent_creation",
      "agent_invocation",
      "model_invocation",
    ]);
    expect(new Set(graph.control_intents.map((c) => c.control_kind))).toEqual(
      new Set(["loop", "conditional", "task_group", "yield"]),
    );
    expect(graph.capability_requirements).toEqual([
      { capability_ref: "search.web.capability.v1", tool_schema_present: true },
    ]);
    expect(graph.context_flow.length).toBeGreaterThan(0);
    expect(graph.hook_bindings).toEqual([
      expect.objectContaining({ scope: "model", phase: "before" }),
    ]);
    expect(graph.imported_program_refs).toEqual([
      expect.objectContaining({ program_ref: "Specialist" }),
    ]);
    expect(Support.diagnostics()).toBeNull();
  });

  it("emits deterministic lexical blocks with typed resume arguments", () => {
    const graph = Support.frontendGraph() as unknown as Graph;
    expect(graph.blocks).toHaveLength(graph.regions.length);
    for (const region of graph.regions) {
      expect(graph.blocks).toContainEqual({
        block_id: `${region.region_id}.block.0`,
        region_id: region.region_id,
        block_arguments: expect.any(Array),
        execution_order: 0,
      });
    }
    const blockWithResume = graph.blocks.find((block) => block.block_arguments.length > 0);
    expect(blockWithResume).toBeDefined();
    for (const valueId of blockWithResume?.block_arguments ?? []) {
      expect(graph.values).toContainEqual(expect.objectContaining({
        value_id: valueId,
        type_ref: "Input",
        origin: "resume_input",
      }));
    }
    expect(graph.source_map.node_spans.every((span) =>
      !span.source_file.startsWith("/") && !span.source_file.includes("/home/"),
    )).toBe(true);
  });

  it("rejects a local binding that shadows a declared Model", () => {
    const shadowedSource = {
      fileName: "shadowed-agent.ts",
      text: `
        import { Agent } from "@apxm/frontend";
        const BoundModel = undefined;
        const Shadowed = Agent({
          name: "Shadowed",
          async run(agent, input) {
            const BoundModel = async (value) => value;
            return await BoundModel(input);
          },
        });
      `,
    };
    expect(() => captureProgram({
      programId: "Shadowed",
      entrypoint: "run",
      inputTypeRef: "Input",
      outputTypeRef: "Output",
      hasDefaultContext: false,
      bindings: new Map([["BoundModel", Model("shadowed.model.v1")]]),
      bindingDeclIds: new Map([["BoundModel", "decl.model.BoundModel"]]),
      source: shadowedSource,
    })).toThrow(CaptureError);
  });

  it("lowers a Tool call to capability.invoke inside ais.loop", () => {
    const air = JSON.parse(Support.canonicalAir()) as {
      semantic_operations: Array<{ op: string }>;
      structural_ir: Array<{ kind: string }>;
    };
    const ops = new Set(air.semantic_operations.map((o) => o.op));
    expect(ops.has("capability.invoke")).toBe(true);
    expect(ops.has("model.call")).toBe(true);
    expect(ops.has("program.new")).toBe(true);
    expect(ops.has("program.invoke")).toBe(true);
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
