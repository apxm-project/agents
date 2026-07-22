/** Gao — a named specialization of the public ConversationalAgent construct. */

import { ConversationalAgent, type TurnSpec } from "./conversational.ts";
import { Hook } from "./hook.ts";
import type { ImportedProgram } from "./agent-program.ts";

const DIGEST_SPECIALIST = "sha256:" + "c".repeat(64);
const DIGEST_HOOK_BEFORE = "sha256:" + "d".repeat(64);
const DIGEST_HOOK_AFTER = "sha256:" + "e".repeat(64);

export class Gao extends ConversationalAgent {
  static build(): Gao {
    const agent = new Gao({
      program_id: "Gao",
      input_type_ref: "GaoInput",
      output_type_ref: "GaoOutput",
      context_type_ref: "GaoContext",
      source_language: "typescript",
    });
    agent.withSpecialist({
      import_ref: {
        program_ref: "Specialist",
        artifact_digest: DIGEST_SPECIALIST,
        entrypoint: "run",
        target_agent_identity_requirement: "specialist-identity",
      } satisfies ImportedProgram,
    });
    agent.defineTurn(
      { capability_ref: "cap.search" } satisfies TurnSpec,
      {
        hooks: [
          Hook.beforeLoop({
            hook_id: "hook.before.turn",
            target_selector: "region.loop.turn",
            handler_ref: "hooks.before_turn",
            handler_digest: DIGEST_HOOK_BEFORE,
            context_type_ref: "GaoContext",
          }).toBinding(),
          Hook.afterModel({
            hook_id: "hook.after.model",
            target_selector: "node.turn.model",
            handler_ref: "hooks.after_model",
            handler_digest: DIGEST_HOOK_AFTER,
            result_type_ref: "ModelResult",
            declaration_order: 1,
          }).toBinding(),
        ],
      },
    );
    return agent;
  }
}

export function gaoConversationalGraph(): Record<string, unknown> {
  return Gao.build().buildGraph();
}
