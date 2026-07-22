// Defines Gao as a TypeScript-only repository example specialization.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { Hook, type AgentProgram, type ImportedProgram } from "@apxm/frontend";
import { ConversationalAgent } from "@apxm/example-agent-conversational";

const SOURCE_PATH = fileURLToPath(new URL("../src/gao.ts", import.meta.url));
const DIGEST_SPECIALIST = `sha256:${"c".repeat(64)}`;
const DIGEST_HOOK_BEFORE = `sha256:${"d".repeat(64)}`;
const DIGEST_HOOK_AFTER = `sha256:${"e".repeat(64)}`;

function bindSourceSpan(
  program: AgentProgram,
  nodeId: string,
  authoredCall: string,
  annotation: string,
): void {
  const lines = readFileSync(SOURCE_PATH, "utf8").split(/\r?\n/);
  const lineIndex = lines.findIndex((text) => text.includes(authoredCall));
  if (lineIndex < 0) throw new Error(`authored call not found: ${authoredCall}`);
  const startColumn = lines[lineIndex].indexOf(authoredCall);
  program.sourceSpan(
    nodeId,
    "gao.ts",
    lineIndex + 1,
    annotation,
    startColumn,
    startColumn + authoredCall.length,
  );
}

export class Gao extends ConversationalAgent {
  constructor() {
    super({
      program_id: "Gao",
      input_type_ref: "GaoInput",
      output_type_ref: "GaoOutput",
      context_type_ref: "GaoContext",
    });
  }

  /** Record Gao through generic composition inside the example-local loop. */
  defineGao(): this {
    const specialist = {
      program_ref: "Specialist",
      artifact_digest: DIGEST_SPECIALIST,
      entrypoint: "run",
      target_agent_identity_requirement: "specialist-identity",
    } satisfies ImportedProgram;
    this.importProgram(specialist);
    this.defineConversation((body) => {
      const { instance } = body.programNew("node.specialist.new", {
        program_ref: specialist.program_ref,
      });
      body.programInvoke("node.specialist.invoke", instance);
    });
    this.bindHook(
      Hook.beforeLoop({
        hook_id: "hook.before.loop",
        target_selector: "region.loop.conversation",
        handler_ref: "hooks.before_loop",
        handler_digest: DIGEST_HOOK_BEFORE,
        context_type_ref: "GaoContext",
      }).toBinding(),
    );
    this.bindHook(
      Hook.afterModel({
        hook_id: "hook.after.model",
        target_selector: "node.model",
        handler_ref: "hooks.after_model",
        handler_digest: DIGEST_HOOK_AFTER,
        result_type_ref: "ModelResult",
      }).toBinding(),
    );
    bindSourceSpan(
      this,
      "node.specialist.new",
      '.programNew("node.specialist.new"',
      "program.new",
    );
    bindSourceSpan(
      this,
      "node.specialist.invoke",
      '.programInvoke("node.specialist.invoke"',
      "program.invoke",
    );
    return this;
  }
}

/** Build the Gao repository example. */
export function buildGao(): Gao {
  return new Gao().defineGao();
}
