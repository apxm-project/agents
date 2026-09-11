// The TypeScript frontend facts the shared conformance corpus cannot state.
//
// Everything about authoring a program — what an Workflow binds, what the
// FrontendGraph says, what the canonical AIR lowers to, and what authoring is
// rejected — is stated once in
// `contracts/vectors/apxm.frontend-conformance.json` and run from the generated
// harness in `frontend-conformance.test.ts`. A fact that only exists in this
// language stays here, because a corpus vector projected into both languages
// could not state it:
//
// * The digest a JavaScript module computes without a Node-only import. Python
//   reaches SHA-256 through `hashlib`, so there is nothing to prove there.
// * A bare typed marker factory. `Event<object>` is not a value in TypeScript,
//   so the mirror of Python's "a subscripted factory is not yet a declaration"
//   is that the marker namespace itself carries no `wait`.
// * Source-token redaction for a module authored outside the workspace. The
//   corpus does not state a fixture's file name — Python reads it back from the
//   real file it was captured from — so only TypeScript, which is handed its
//   source, can state a file name that escapes the workspace.
// * The generated runtime-evidence binding. It is a generated contract binding
//   rather than an authored Workflow, so it has no source fixture to capture and no
//   FrontendGraph to compare.
// * A `Skill` stating both an entry and inline text. The corpus's
//   `declaration_rejections` vocabulary states one reference per marker and
//   calls the marker with it, so it can state the skill that names neither
//   source — and does — but not the one that names two. The Python frontend
//   carries the same test for the same reason.
// * The scope a Hook states, resolved from the vocabulary symbol the module
//   imported. Python holds the symbol's value by the time the decorator runs, so
//   only TypeScript, which reads the scope back out of the AST, can resolve one
//   wrongly or refuse one it cannot resolve.
// * A Hook whose target names nothing. A Python Hook binds through the module
//   globals its Workflow resolves, so a Hook nobody declares a target for is a fact
//   about a whole module rather than about one authored program, and a corpus
//   vector authors its programs inside one shared module. The Python frontend
//   carries the same test for the same reason.

import { describe, expect, it } from "vitest";

import { source } from "../src/node.ts";
import { Workflow, Capability, Context, Event, Model, Skill, Tool } from "../src/index.ts";
import { CaptureError, captureProgram } from "../src/capture.ts";
import { declaredSoFar } from "../src/declared.ts";
import { decodeFact } from "../src/generated/runtime-evidence.ts";
import { stableDigest } from "../src/markers.ts";
import {
  HOOK_SCOPE_UNRESOLVED,
  HOOK_TARGET_UNRESOLVED,
  SKILL_ENTRY_PATH_NOT_CANONICAL,
  SKILL_SOURCE_AMBIGUOUS,
} from "../src/generated/diagnostics.ts";
import {
  HOOK_SCOPE_CAPABILITY,
  INPUT_CONTRACT_ACCEPTS_EMPTY_OBJECT,
} from "../src/generated/frontend-graph.ts";
import { ASK, Ask } from "../src/generated/permissions.ts";
import { READ } from "../src/capabilities.ts";
import type { CallIntent, Declaration, ProgramDefinition, Value } from "../src/generated/frontend-records.ts";

source(import.meta.url);

type Input = any;
type Output = any;

void Workflow;

function capturedInputContract(inputType: string, config = "", body = "return input;"): unknown {
  return captureProgram({
    programId: "InputContractAgent",
    entrypoint: "InputContractAgent",
    declared: [],
    source: {
      fileName: "input-contract.ts",
      text: `
        import { Workflow } from "@apxm/frontend";
        type OptionalInput = { label?: string };
        type ListInput = string[];
        type TupleInput = [string];
        type ArrayInput = Array<string>;
        type ReadonlyArrayInput = ReadonlyArray<string>;
        type MappedInput = { [key in "label"]?: string };
        type GenericInput<T> = T;
        type DynamicUnknownInput = GenericInput<unknown>;
        type DynamicAnyInput = GenericInput<any>;
        type ConditionalOptionalInput = unknown extends string ? { required: string } : {};
        type ConditionalRequiredInput = string extends string ? { required: string } : {};
        type RecursiveInput = { child?: RecursiveInput };
        const InputContractAgent = Workflow<${inputType}, unknown>({
          name: "InputContractAgent",
          ${config}
          async run(agent, input) { ${body} },
        });
      `,
    },
  });
}

type CapturedEventGraph = {
  declarations: Declaration[];
  program_definitions: ProgramDefinition[];
  call_intents: CallIntent[];
  values: Value[];
};

function capturedEventContract(reference = "input.event", payload = "Payload", referenceType = "Payload"): CapturedEventGraph {
  return captureProgram({
    programId: "EventWorkflow", entrypoint: "EventWorkflow",
    declared: [Event<{ reference: string; approved: boolean }>("event.submitted"), Capability(READ)],
    source: { fileName: "event-contract.ts", text: `
      import { Workflow, Event, type EventRef, Capability } from "@apxm/frontend";
      type Payload = { reference: string; approved: boolean };
      type Input = { event: EventRef<${referenceType}> };
      const Submitted = Event<${payload}>("event.submitted");
      const Read = Capability<Payload, Payload>("read");
      const EventWorkflow = Workflow<Input, Payload>({ async run(agent, input) {
        const result = await Submitted.wait(${reference});
        return await Read(result);
      }});
    ` },
  }) as unknown as CapturedEventGraph;
}

describe("typed Event references", () => {
  it("captures an admitted reference operand and finite payload schema", () => {
    const graph = capturedEventContract();
    expect(graph.declarations.find((value) => value.decl_kind === "event_type")!.payload_schema).toEqual({
      type: "object", properties: { reference: { type: "string" }, approved: { type: "boolean" } },
      required: ["reference", "approved"], additionalProperties: false,
    });
    expect(graph.program_definitions[0].input_schema!.properties!.event).toEqual({
      type: "object", properties: { event_id: { type: "string" }, generation: { type: "integer" } },
      required: ["event_id", "generation"], additionalProperties: false,
    });
    const wait = graph.call_intents.find((call) => call.intent_kind === "event_wait")!;
    const operands = wait.operand_values!;
    expect(operands).toHaveLength(1);
    expect(graph.values.find((value) => value.value_id === operands[0])!.type_ref).toBe("EventRef");
  });
  it.each(["", "'event.submitted'", "{event_id:'evt-forged',generation:1}", "input.event as any"])("refuses an untyped or manufactured reference: %s", (reference) => {
    expect(() => capturedEventContract(reference)).toThrow(CaptureError);
  });
  it("refuses mismatched and unsupported payload declarations", () => {
    expect(() => capturedEventContract("input.event", "Payload", "string")).toThrow(CaptureError);
    expect(() => capturedEventContract("input.event", "unknown")).toThrow(CaptureError);
  });
  it("gives inline payload types a stable declaration identity", () => {
    const graph = capturedEventContract("input.event", "{ reference: string; approved: boolean }");
    const declaration = graph.declarations.find((value) => value.decl_kind === "event_type")!;
    expect(declaration.output_type_ref).toBe("decl.event.Submitted.payload");
    expect(declaration.payload_schema!.type).toBe("object");
  });
});

/** One module whose single Hook states `scope` exactly as `declared` writes it. */
function capturedScope(
  programId: string,
  modelRef: string,
  declared: { imports: string; scope: string },
): { hook_bindings: Array<{ scope: string }> } {
  return captureProgram({
    programId,
    entrypoint: "run",
    declared: declaredSoFar(),
    source: {
      fileName: `${programId}.ts`,
      text: `
        import { Workflow, Hook, Model } from "@apxm/frontend";
        ${declared.imports}
        const ScopedModel = Model<Input, Output>("${modelRef}");
        const ${programId} = Workflow<Input, Output>({
          name: "${programId}",
          async run(agent, input) {
            return await ScopedModel(input);
          },
        });
        const ScopedHook = Hook.before({
          agent: ${programId},
          target: ScopedModel,
          scope: ${declared.scope},
          async run(agent) {},
        });
      `,
    },
  }) as unknown as { hook_bindings: Array<{ scope: string }> };
}

describe("language-local TypeScript frontend facts", () => {
  it("stamps empty-object eligibility only when the checker proves it", () => {
    const accepted = [
      "unknown",
      "{}",
      "{ label?: string }",
      "OptionalInput",
      "MappedInput",
      "DynamicUnknownInput",
      "ConditionalOptionalInput",
    ];
    for (const inputType of accepted) {
      const graph = capturedInputContract(inputType) as {
        program_definitions: Array<{ input_contract?: string }>;
      };
      expect(graph.program_definitions[0]?.input_contract).toBe(
        INPUT_CONTRACT_ACCEPTS_EMPTY_OBJECT,
      );
    }

    for (const inputType of [
      "any",
      "{ label: string }",
      "string",
      "string[]",
      "ListInput",
      "TupleInput",
      "ArrayInput",
      "ReadonlyArrayInput",
      "never",
      "{ label?: string } | string",
      "MissingInput",
      "DynamicAnyInput",
      "ConditionalRequiredInput",
    ]) {
      const graph = capturedInputContract(inputType) as {
        program_definitions: Array<{ input_contract?: string }>;
      };
      expect(graph.program_definitions[0]?.input_contract).toBeUndefined();
    }
  });

  it("does not let authored metadata forge eligibility", () => {
    const graph = capturedInputContract(
      "{ required: string }",
      'input_contract: "accepts_empty_object",',
    ) as { program_definitions: Array<{ input_contract?: string }> };
    expect(graph.program_definitions[0]?.input_contract).toBeUndefined();
  });

  it("projects resolved JSON input types into closed compiler-owned schemas", () => {
    const graph = capturedInputContract(
      "{ reference: string; count: number; accepted: boolean; absent: null; labels: string[]; details?: { note?: string; enabled?: boolean } }",
    ) as { program_definitions: Array<{ input_schema?: unknown }> };
    expect(graph.program_definitions[0]?.input_schema).toEqual({
      type: "object",
      additionalProperties: false,
      required: ["reference", "count", "accepted", "absent", "labels"],
      properties: {
        reference: { type: "string" },
        count: { type: "number" },
        accepted: { type: "boolean" },
        absent: { type: "null" },
        labels: { type: "array", items: { type: "string" } },
        details: {
          type: "object", additionalProperties: false, required: [],
          properties: { note: { type: "string" }, enabled: { type: "boolean" } },
        },
      },
    });
    for (const inputType of ["OptionalInput", "MappedInput"]) {
      const graph = capturedInputContract(inputType) as { program_definitions: Array<{ input_schema?: unknown }> };
      expect(graph.program_definitions[0]?.input_schema).toEqual({
        type: "object", additionalProperties: false, required: [], properties: { label: { type: "string" } },
      });
    }
    for (const inputType of ["ListInput", "ArrayInput", "ReadonlyArrayInput"]) {
      const graph = capturedInputContract(inputType) as { program_definitions: Array<{ input_schema?: unknown }> };
      expect(graph.program_definitions[0]?.input_schema).toEqual({ type: "array", items: { type: "string" } });
    }
  });

  it("leaves unsupported input types without a schema instead of widening them", () => {
    for (const inputType of [
      "unknown", "any", "never", "MissingInput", "DynamicAnyInput", "RecursiveInput",
      '"literal"', "123", "true", "bigint", "undefined", "symbol", "object",
      "TupleInput", "Promise<string>", "{ value: string | null }", "{ value: unknown }",
      "{ [key: string]: string }", "{ left: string } & { right: number }", "() => string", "Date",
    ]) {
      const graph = capturedInputContract(inputType) as { program_definitions: Array<{ input_schema?: unknown }> };
      expect(graph.program_definitions[0]?.input_schema).toBeUndefined();
    }
  });

  it("ignores authored metadata that attempts to replace the checked input schema", () => {
    const graph = capturedInputContract(
      "{ reference: string }",
      'input_schema: { type: "object", properties: {}, required: [], additionalProperties: true },',
    ) as { program_definitions: Array<{ input_schema?: unknown }> };
    expect(graph.program_definitions[0]?.input_schema).toEqual({
      type: "object", additionalProperties: false, required: ["reference"], properties: { reference: { type: "string" } },
    });
  });

  it("captures named pure data without adding an effect operation", () => {
    const graph = capturedInputContract("{ reference: string }", "", "const mapped = {reference: input.reference}; return mapped;") as { values: Array<{ expression?: unknown }>; call_intents: unknown[] };
    expect(graph.call_intents).toEqual([]);
    expect(graph.values.some((value) => (value.expression as { kind?: string })?.kind === "object")).toBe(true);
  });

  it.each([
    "let mapped = {}; return mapped;",
    'const mapped = {}; mapped.value = "changed"; return mapped;',
    "const mapped = []; mapped.push(1); return mapped;",
    "const mapped = agent.context; return mapped;",
    "if (input.reference) { const mapped = {}; } return mapped;",
    "const mapped = missing; return mapped;",
    "const mapped = input; input.reference = 'changed'; return mapped;",
    "const mapped = {__proto__: input}; return mapped;",
  ])("refuses mutation, mutable Context and escaped pure local bindings: %s", (body) => {
    expect(() => capturedInputContract("{ reference: string }", "", body)).toThrow(CaptureError);
  });

  it("emits standard SHA-256 digests without a Node-only root import", () => {
    expect(stableDigest("abc")).toBe(
      "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
  });

  it("keeps the marker namespace free of the declaration's own members", () => {
    const event = Event<object>("event.session.input");
    expect(event.targetRef).toBe("event.session.input");
    expect((Event as unknown as { wait?: unknown }).wait).toBeUndefined();
  });

  it("freezes marker records against JavaScript any-cast mutation", () => {
    const model = Model<Input, Output>("immutable.model");
    const tool = Tool<Input, Output>(READ, { permission: Ask("write") });
    const capability = Capability<Input, Output>(READ, { permission: Ask("call") });
    const event = Event<object>("event.immutable");
    const skill = Skill("immutable", { text: "Read this." });
    const context = Context<{ requestId: string }>();

    for (const record of [model, tool, capability, event, skill, context]) {
      expect(Object.isFrozen(record)).toBe(true);
      expect(() =>
        Object.defineProperty(record, "targetRef", { value: "evil.ref" }),
      ).toThrow();
    }
    expect(Object.isFrozen(tool.permission)).toBe(true);
    expect(Object.isFrozen(capability.permission)).toBe(true);
    expect(Object.isFrozen(skill.instructionSource)).toBe(true);
    expect(() =>
      Object.defineProperty(tool.permission as object, "reason", { value: "evil" }),
    ).toThrow();
    expect(() =>
      Object.defineProperty(skill.instructionSource as object, "text", { value: "evil" }),
    ).toThrow();
  });

  it("holds a Skill to one instruction source", () => {
    expect(() =>
      Skill("review", { entry: "skills/review/SKILL.md", text: "Review carefully." }),
    ).toThrow(SKILL_SOURCE_AMBIGUOUS);
  });

  it("holds a file-carried Skill to the path its id resolves to", () => {
    expect(() => Skill("review", { entry: "prompts/review.md" })).toThrow(
      SKILL_ENTRY_PATH_NOT_CANONICAL,
    );
  });

  it("keeps JavaScript permission values on the generated closed vocabulary", () => {
    expect(() =>
      Tool<Input, Output>(READ, { permission: "allow" as never }),
    ).toThrow("generated Allow, Ask, or Deny");
    expect(() =>
      Tool<Input, Output>(READ, { permission: Ask("") }),
    ).toThrow("reason must be a non-empty string");
    expect(ASK).toBe("ask");
  });

  it("refuses a Hook target no declaration names", () => {
    void Model<Input, Output>("stray.hook.model");
    let capturedError: unknown;
    try {
      captureProgram({
        programId: "StrayHookTarget",
        entrypoint: "run",
        declared: declaredSoFar(),
        source: {
          fileName: "stray-hook.ts",
          text: `
            import { Workflow, Hook, Model } from "@apxm/frontend";
            const StrayModel = Model<Input, Output>("stray.hook.model");
            const StrayHookTarget = Workflow<Input, Output>({
              name: "StrayHookTarget",
              async run(agent, input) {
                return await StrayModel(input);
              },
            });
            const StrayHook = Hook.before({
              agent: StrayHookTarget,
              target: NoDeclarationNamesThis,
              scope: "model",
              async run(agent) {},
            });
          `,
        },
      });
    } catch (error) {
      capturedError = error;
    }
    expect(capturedError).toBeInstanceOf(CaptureError);
    expect((capturedError as CaptureError).code).toBe(HOOK_TARGET_UNRESOLVED);
  });

  it("redacts source tokens that escape the author workspace", () => {
    void Model<Input, Output>("escaping.source.model");
    const graph = captureProgram({
      programId: "EscapingSource",
      entrypoint: "run",
      declared: declaredSoFar(),
      source: {
        fileName: "../../outside-workspace/agent.ts",
        text: `
          import { Workflow, Model } from "@apxm/frontend";
          const EscapingModel = Model<Input, Output>("escaping.source.model");
          const EscapingSource = Workflow<Input, Output>({
            async run(agent, input) {
              return await EscapingModel(input);
            },
          });
        `,
      },
    }) as unknown as { source_map: { node_spans: Array<{ source_file: string }> } };

    expect(graph.source_map.node_spans).toHaveLength(1);
    expect(graph.source_map.node_spans[0]?.source_file).toBe("<agent>");
  });

  it("resolves an imported scope symbol to the scope it names", () => {
    void Model<Input, Output>("scope.symbol.model");
    const graph = capturedScope("ScopeSymbol", "scope.symbol.model", {
      imports: `import { CAPABILITY } from "@apxm/frontend/scopes";`,
      scope: "CAPABILITY",
    });
    expect(graph.hook_bindings[0]?.scope).toBe(HOOK_SCOPE_CAPABILITY);
  });

  it("resolves an aliased scope symbol the same way", () => {
    void Model<Input, Output>("scope.alias.model");
    const graph = capturedScope("ScopeAlias", "scope.alias.model", {
      imports: `import { CAPABILITY as Wraps } from "@apxm/frontend/scopes";`,
      scope: "Wraps",
    });
    expect(graph.hook_bindings[0]?.scope).toBe(HOOK_SCOPE_CAPABILITY);
  });

  it("refuses a scope it cannot resolve rather than defaulting to node", () => {
    void Model<Input, Output>("scope.dynamic.model");
    expect(() =>
      capturedScope("ScopeDynamic", "scope.dynamic.model", {
        imports: "const chosen = process.env.SCOPE;",
        scope: "chosen",
      }),
    ).toThrow(HOOK_SCOPE_UNRESOLVED);
  });

  it("refuses a scope literal the vocabulary does not mint", () => {
    void Model<Input, Output>("scope.invented.model");
    expect(() =>
      capturedScope("ScopeInvented", "scope.invented.model", {
        imports: "",
        scope: `"capabilities"`,
      }),
    ).toThrow(HOOK_SCOPE_UNRESOLVED);
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

  it("preserves specialized model-attempt identity while rejecting bare facts", () => {
    const fact = decodeFact({
      fact_id: "attempt.1",
      event_sequence: 2,
      fact_kind: "attempt.recorded",
      program_invocation_id: "invocation.1",
      node_execution_id: "node-execution.1",
      air_node_id: "node.model",
      attempt_id: "attempt.1",
      attempt_index: 0,
      model_effect_id: "effect.1",
      request_digest: `sha256:${"1".repeat(64)}`,
      model_target_ref: "model-target.1",
      model_target_digest: `sha256:${"2".repeat(64)}`,
      model_deployment_ref: "deployment.1",
      exact_port_binding_digest: `sha256:${"3".repeat(64)}`,
      target_commitment_digest: `sha256:${"4".repeat(64)}`,
      generation_cohort_digest: `sha256:${"5".repeat(64)}`,
      target_generation: 1,
      target_port_contract_digest: `sha256:${"6".repeat(64)}`,
      target_composition_digest: `sha256:${"7".repeat(64)}`,
      native_input_tokens: 3,
      native_output_tokens: 5,
    });
    if (fact.fact_kind !== "attempt.recorded") throw new Error("lost attempt identity");
    expect(fact.attempt_id).toBe("attempt.1");
    expect(() => decodeFact({ fact_id: "attempt.2", event_sequence: 3, fact_kind: "attempt.recorded" })).toThrow();
  });
});

describe("host-fulfilled capability minting", () => {
  it("mints only the host capabilities the trusted bridge declared", async () => {
    const { Capability } = await import("../src/index.ts");
    const { declareHostCapabilities } = await import("../src/host.ts");

    declareHostCapabilities([]);
    expect(() => Capability("host:notes.search")).toThrow(
      /the package manifest does not declare as[\s\S]*declares none/,
    );

    declareHostCapabilities(["notes.search"]);
    expect(Capability("host:notes.search").targetRef).toBe("host:notes.search");
    expect(() => Capability("host:notes.append")).toThrow(
      /host:notes\.search/,
    );

    declareHostCapabilities([]);
  });
});

type CapturedOwnerGraph = {
  values: Value[];
  control_intents: Array<{ control_kind: string; result_value?: string; operand_values?: string[] }>;
};

function capturedOwnerRequest(call: string): CapturedOwnerGraph {
  return captureProgram({
    programId: "Consent", entrypoint: "Consent", declared: [],
    source: { fileName: "owner-request.ts", text: `
      import { Workflow } from "@apxm/frontend";
      type Input = { message: string };
      type Amount = { amount: number; note?: string };
      const Consent = Workflow<Input, unknown>({ async run(agent, input) {
        const reply = await ${call};
        return { outcome: reply.outcome };
      }});
    ` },
  }) as unknown as CapturedOwnerGraph;
}

describe("owner requests", () => {
  it("captures ask_owner as a typed yield with literal choices", () => {
    const graph = capturedOwnerRequest(
      'agent.ask_owner({ prompt: input.message, choices: [{ id: "send", label: "Send it" }, { id: "hold", label: "Hold" }], expires_in_seconds: 3600 })',
    );
    const [request] = graph.values.filter((value) => value.type_ref === "OwnerRequest");
    expect(request?.origin).toBe("literal");
    const expression = request?.expression as unknown as { fields: Array<{ name: string; value: unknown }> };
    const fields = Object.fromEntries(expression.fields.map((field) => [field.name, field.value]));
    expect(Object.keys(fields)).toEqual(["prompt", "answer", "expires_in_seconds"]);
    expect((fields.prompt as { kind: string }).kind).toBe("projection");
    expect(fields.expires_in_seconds).toEqual({ kind: "integer", value: 3600 });
    expect(fields.answer).toEqual({
      kind: "object",
      fields: [
        { name: "mode", value: { kind: "string", value: "choice" } },
        { name: "choices", value: { kind: "array", items: [
          { kind: "object", fields: [{ name: "id", value: { kind: "string", value: "send" } }, { name: "label", value: { kind: "string", value: "Send it" } }] },
          { kind: "object", fields: [{ name: "id", value: { kind: "string", value: "hold" } }, { name: "label", value: { kind: "string", value: "Hold" } }] },
        ] } },
      ],
    });
    const [resume] = graph.values.filter((value) => value.origin === "resume_input");
    expect(resume?.type_ref).toBe("OwnerAnswer");
    const [yielded] = graph.control_intents.filter((intent) => intent.control_kind === "yield");
    expect(yielded?.result_value).toBe(resume?.value_id);
    expect(yielded?.operand_values).toEqual([request?.value_id]);
  });

  it("projects a typed Answer into the closed schema with sorted keys", () => {
    const graph = capturedOwnerRequest('agent.ask_owner<Amount>({ prompt: "How much?", expires_in_seconds: 60 })');
    const [request] = graph.values.filter((value) => value.type_ref === "OwnerRequest");
    const expression = request?.expression as unknown as { fields: Array<{ name: string; value: { fields: Array<{ name: string; value: unknown }> } }> };
    const answer = expression.fields.find((field) => field.name === "answer")!.value;
    expect(answer.fields[0]).toEqual({ name: "mode", value: { kind: "string", value: "typed" } });
    const schema = answer.fields[1]!.value as { fields: Array<{ name: string }> };
    expect(schema.fields.map((field) => field.name)).toEqual(["additionalProperties", "properties", "required", "type"]);
  });

  it("refuses requests outside the closed shape", () => {
    for (const call of [
      'agent.ask_owner({ prompt: "Send?", expires_in_seconds: 60 })',
      'agent.ask_owner<Amount>({ prompt: "Send?", choices: [{ id: "a", label: "A" }], expires_in_seconds: 60 })',
      'agent.ask_owner({ prompt: "Send?", choices: [{ id: "a", label: "A" }] })',
      'agent.ask_owner({ prompt: "Send?", choices: [{ id: "a", label: "A" }], expires_in_seconds: 0 })',
      'agent.ask_owner({ prompt: "Send?", choices: [{ id: "a" }], expires_in_seconds: 60 })',
      'agent.ask_owner({ prompt: "Send?", choices: input.message, expires_in_seconds: 60 })',
      'agent.ask_owner({ prompt: 7, choices: [{ id: "a", label: "A" }], expires_in_seconds: 60 })',
      'agent.ask_owner({ prompt: "Send?", choices: [{ id: "a", label: "A" }], expires_in_seconds: 60, schema_digest: "sha256:00" })',
      'agent.ask_owner<string[]>({ prompt: "Send?", expires_in_seconds: 60 }, {})',
    ]) {
      expect(() => capturedOwnerRequest(call), call).toThrow(CaptureError);
    }
  });
});
