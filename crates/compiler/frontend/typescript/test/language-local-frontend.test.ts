// The TypeScript frontend facts the shared conformance corpus cannot state.
//
// Everything about authoring a program — what an Agent binds, what the
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
//   rather than an authored Agent, so it has no source fixture to capture and no
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
//   globals its Agent resolves, so a Hook nobody declares a target for is a fact
//   about a whole module rather than about one authored program, and a corpus
//   vector authors its programs inside one shared module. The Python frontend
//   carries the same test for the same reason.

import { describe, expect, it } from "vitest";

import { source } from "../src/node.ts";
import { Agent, Capability, Context, Event, Model, Skill, Tool } from "../src/index.ts";
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
import { HOOK_SCOPE_CAPABILITY } from "../src/generated/frontend-graph.ts";
import { ASK, Ask } from "../src/generated/permissions.ts";
import { READ } from "../src/capabilities.ts";

source(import.meta.url);

type Input = any;
type Output = any;

void Agent;

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
        import { Agent, Hook, Model } from "@apxm/frontend";
        ${declared.imports}
        const ScopedModel = Model<Input, Output>("${modelRef}");
        const ${programId} = Agent<Input, Output>({
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
            import { Agent, Hook, Model } from "@apxm/frontend";
            const StrayModel = Model<Input, Output>("stray.hook.model");
            const StrayHookTarget = Agent<Input, Output>({
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
          import { Agent, Model } from "@apxm/frontend";
          const EscapingModel = Model<Input, Output>("escaping.source.model");
          const EscapingSource = Agent<Input, Output>({
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
