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
// * A Hook whose target names nothing. A Python Hook binds through the module
//   globals its Agent resolves, so a Hook nobody declares a target for is a fact
//   about a whole module rather than about one authored program, and a corpus
//   vector authors its programs inside one shared module. The Python frontend
//   carries the same test for the same reason.

import { describe, expect, it } from "vitest";

import { source } from "../src/node.ts";
import { Agent, Event, Model, Skill } from "../src/index.ts";
import { captureProgram } from "../src/capture.ts";
import { declaredSoFar } from "../src/declared.ts";
import { decodeFact } from "../src/generated/runtime-evidence.ts";
import { stableDigest } from "../src/markers.ts";
import {
  HOOK_TARGET_UNRESOLVED,
  SKILL_ENTRY_PATH_NOT_CANONICAL,
  SKILL_SOURCE_AMBIGUOUS,
} from "../src/generated/diagnostics.ts";

source(import.meta.url);

type Input = any;
type Output = any;

void Agent;

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

  it("refuses a Hook target no declaration names", () => {
    void Model<Input, Output>("stray.hook.model");
    expect(() =>
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
      }),
    ).toThrow(HOOK_TARGET_UNRESOLVED);
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
