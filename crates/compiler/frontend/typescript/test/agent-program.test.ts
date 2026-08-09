// Source-first TypeScript authoring produces the typed FrontendGraph.

import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import "../src/node.ts";
import { Agent, Capability, Context, Event, Hook, Model, TaskGroup, Tool } from "../src/index.ts";
import { CaptureError, captureProgram } from "../src/capture.ts";
import { decodeFact } from "../src/generated/runtime-evidence.ts";
import { stableDigest } from "../src/markers.ts";

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
const InitialCtx = Context({ messages: [] as string[] });
const InitialModel = Model("initial.model.v1");

const InitialContextAgent = Agent({
  name: "InitialContextAgent",
  source,
  context: InitialCtx,
  use: { InitialModel },
  async run(agent, incoming) {
    agent.context = { messages: [] };
    return await InitialModel(incoming);
  },
});

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
  program_definitions: Array<{
    program_id: string;
    entrypoint: string;
    input_type_ref: string;
    output_type_ref: string;
  }>;
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
  call_intents: Array<{ intent_kind: string; operand_values?: string[] }>;
  control_intents: Array<{ control_kind: string }>;
  context_flow: Array<{ from_node: string; to_node: string }>;
  hook_bindings: Array<{ scope: string; phase: string }>;
  imported_program_refs: Array<{ program_ref: string }>;
  capability_requirements: Array<{ capability_ref: string; tool_schema_present: boolean }>;
  source_map: { node_spans: Array<{ source_file: string }> };
};

describe("source-first TypeScript authoring", () => {
  it("emits standard SHA-256 digests without a Node-only root import", () => {
    expect(stableDigest("abc")).toBe(
      "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });

  it("binds a minimal one-shot Model agent", () => {
    const graph = Summarizer.frontendGraph() as unknown as Graph;
    expect(graph.schema_version).toBe("apxm.frontend-graph.v2");
    expect(graph.declarations.map((d) => d.decl_kind)).toEqual(["model_binding"]);
    expect(graph.call_intents.map((c) => c.intent_kind)).toEqual(["model_invocation"]);
    expect(Summarizer.diagnostics()).toBeNull();
  });

  it("lowers the minimal agent to a registered model.call", () => {
    const air = JSON.parse(Summarizer.canonicalAir()) as {
      semantic_operations: Array<{
        op: string;
        operands: Array<{ slot: string; value_id: string }>;
      }>;
    };
    expect(air.semantic_operations.map((o) => o.op)).toEqual(["model.call"]);
    expect(air.semantic_operations[0].operands.some((o) => o.slot === "request")).toBe(true);
    expect(air.semantic_operations[0].operands).toContainEqual({
      slot: "model_ref",
      value_id: "summarizer.model.v1",
      type_ref: "ModelTargetRef",
    });
  });

  it("binds an initial Context assignment to the lexical region entry", () => {
    const graph = InitialContextAgent.frontendGraph() as unknown as Graph;
    expect(graph.context_flow).toHaveLength(1);
    expect(graph.context_flow[0]?.from_node).toBe("InitialContextAgent.body");
    expect(graph.context_flow[0]?.to_node).toMatch(/^InitialContextAgent\.model_invocation\./);
    expect(InitialContextAgent.diagnostics()).toBeNull();

    const air = JSON.parse(InitialContextAgent.canonicalAir()) as {
      context_flow: Array<{ from_node: string }>;
    };
    expect(air.context_flow[0]?.from_node).toBe("InitialContextAgent.body");
  });

  it("rejects display aliases and handler objects while preserving constructed event refs", () => {
    for (const marker of [Model, Tool, Capability, Event]) {
      for (const invalid of ["", "default", "model.default", "support", "search-web"]) {
        expect(() => marker(invalid)).toThrow(/exact typed reference/);
      }
    }
    expect(() => Tool({ run() {} } as unknown as string)).toThrow(/exact typed reference/);
    expect(() => Capability({ run() {} } as unknown as string)).toThrow(/exact typed reference/);

    const event = Event<object>("event.session.input.v1");
    expect(event.targetRef).toBe("event.session.input.v1");
    expect((Event as unknown as { wait?: unknown }).wait).toBeUndefined();
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
      expect.objectContaining({
        scope: "model",
        phase: "before",
        handler_ref: "SupportPolicy",
      }),
    ]);
    expect(graph.declarations).toContainEqual(
      expect.objectContaining({ decl_id: "decl.context.Context" }),
    );
    expect(graph.imported_program_refs).toEqual([
      expect.objectContaining({ program_ref: "Specialist" }),
    ]);
    expect(Support.diagnostics()).toBeNull();
  });

  it("reuses named values and allocates instance identities like the Python frontend", () => {
    const graph = Support.frontendGraph() as unknown as Graph;
    const creation = graph.values.find((value) =>
      value.origin === "call_result" &&
      value.type_ref === "ProgramInstanceRef"
    );
    expect(creation?.value_id).toMatch(/^Support\.instance\.\d+$/);

    const invoke = graph.call_intents.find((call) => call.intent_kind === "agent_invocation");
    expect(invoke?.operand_values).toContain("Support.param.input");

    expect(graph.program_definitions[0]).toEqual(
      expect.objectContaining({
        program_id: "Support",
        entrypoint: "Support",
        input_type_ref: "Input",
        output_type_ref: "Output",
      }),
    );
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

  it("redacts source tokens that escape the author workspace", () => {
    const graph = captureProgram({
      programId: "EscapingSource",
      entrypoint: "run",
      inputTypeRef: "Input",
      outputTypeRef: "Output",
      hasDefaultContext: false,
      bindings: new Map([["SummarizerModel", SummarizerModel]]),
      bindingDeclIds: new Map([["SummarizerModel", "decl.model.SummarizerModel"]]),
      source: {
        fileName: "../../outside-workspace/agent.ts",
        text: `
          import { Agent, Model } from "@apxm/frontend";
          const SummarizerModel = Model("summarizer.model.v1");
          const EscapingSource = Agent({
            async run(agent, input) {
              return await SummarizerModel(input);
            },
          });
        `,
      },
    }) as unknown as Graph;

    expect(graph.source_map.node_spans).toHaveLength(1);
    expect(graph.source_map.node_spans[0]?.source_file).toBe("<agent>");
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

  it("rejects an unresolved or effectful call standing in a value position", () => {
    const captureValuePosition = (body: string) =>
      captureProgram({
        programId: "ValuePosition",
        entrypoint: "run",
        inputTypeRef: "Input",
        outputTypeRef: "Output",
        hasDefaultContext: false,
        bindings: new Map([["BoundModel", Model("value.position.model.v1")]]),
        bindingDeclIds: new Map([["BoundModel", "decl.model.BoundModel"]]),
        source: {
          fileName: "value-position-agent.ts",
          text: `
            import { Agent, Model } from "@apxm/frontend";
            const BoundModel = Model("value.position.model.v1");
            const ValuePosition = Agent({
              name: "ValuePosition",
              use: { BoundModel },
              async run(agent, input) {
                ${body}
              },
            });
          `,
        },
      });

    // A returned call the frontend cannot resolve was previously dropped, so
    // the graph silently lost the author's whole result expression.
    expect(() => captureValuePosition("return Unresolved(input);")).toThrow(
      /call target 'Unresolved' is not a bound Context schema/,
    );
    // An unresolved call nested in an operand was previously folded into an
    // untyped literal operand, hiding it from lowering and from admission.
    expect(() =>
      captureValuePosition("return await BoundModel(Unresolved(input));"),
    ).toThrow(/call target 'Unresolved' is not a bound Context schema/);
    // A typed effect is an effect wherever it is written: reaching it without
    // await must not degrade it to data.
    expect(() => captureValuePosition("return BoundModel(input);")).toThrow(
      /'BoundModel' is a typed effect and is called with await/,
    );
  });

  it("rejects predicate integers outside the shared safe domain", () => {
    expect(() => captureProgram({
      programId: "UnsafeIntegerPredicate",
      entrypoint: "run",
      inputTypeRef: "Input",
      outputTypeRef: "Output",
      hasDefaultContext: false,
      bindings: new Map(),
      bindingDeclIds: new Map(),
      source: {
        fileName: "unsafe-integer-agent.ts",
        text: `
          import { Agent } from "@apxm/frontend";
          const UnsafeIntegerPredicate = Agent({
            name: "UnsafeIntegerPredicate",
            async run(agent, input) {
              if (input.count === 9007199254740992) return input;
              return input;
            },
          });
        `,
      },
    })).toThrow(/safe-integer/);
  });

  it("preserves negative authored integer expressions at the safe boundary", () => {
    const graph = captureProgram({
      programId: "NegativeIntegerValue",
      entrypoint: "run",
      inputTypeRef: "Input",
      outputTypeRef: "Output",
      hasDefaultContext: false,
      bindings: new Map([["BoundModel", Model("negative.value.model.v1")]]),
      bindingDeclIds: new Map([["BoundModel", "decl.model.BoundModel"]]),
      source: {
        fileName: "negative-value-agent.ts",
        text: `
          import { Agent, Model } from "@apxm/frontend";
          const BoundModel = Model("negative.value.model.v1");
          const NegativeIntegerValue = Agent({
            name: "NegativeIntegerValue",
            async run(agent, input) {
              return await BoundModel(-9007199254740991);
            },
          });
        `,
      },
    }) as unknown as Graph;
    const value = graph.values.find((candidate) => candidate.origin === "literal");
    expect(value?.expression).toEqual({ kind: "integer", value: -9007199254740991 });
  });

  it("rejects effect calls that would silently discard authored operands", () => {
    expect(() => captureProgram({
      programId: "MultipleOperands",
      entrypoint: "run",
      inputTypeRef: "Input",
      outputTypeRef: "Output",
      hasDefaultContext: false,
      bindings: new Map([["BoundModel", Model("multiple.operands.model.v1")]]),
      bindingDeclIds: new Map([["BoundModel", "decl.model.BoundModel"]]),
      source: {
        fileName: "multiple-operands-agent.ts",
        text: `
          import { Agent, Model } from "@apxm/frontend";
          const BoundModel = Model("multiple.operands.model.v1");
          const MultipleOperands = Agent({
            name: "MultipleOperands",
            async run(agent, input) {
              return await BoundModel(input, input);
            },
          });
        `,
      },
    })).toThrow(/exactly one authored operand/);
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
