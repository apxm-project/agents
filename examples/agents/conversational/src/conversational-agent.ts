// Defines the example-local conversational specialization over generic APIs.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { AgentProgram } from "@apxm/frontend";

const SOURCE_PATH = fileURLToPath(new URL("../src/conversational-agent.ts", import.meta.url));

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
    "conversational-agent.ts",
    lineIndex + 1,
    annotation,
    startColumn,
    startColumn + authoredCall.length,
  );
}

export class ConversationalAgent extends AgentProgram {
  /** Record one ordinary structured conversation loop. */
  defineConversation(compose?: (body: this) => unknown): this {
    this.loop("region.loop.conversation", (body) => {
      body
        .modelCall("node.model", "model.default")
        .capabilityInvoke("node.capability", "cap.search");
      compose?.(body);
      body.awaitEvent("node.await", "event.conversation.input");
    });
    this.contextFlow({
      from_node: "node.model",
      to_node: "node.capability",
      context_type_ref: "ConversationContext",
    });
    bindSourceSpan(
      this,
      "node.model",
      '.modelCall("node.model"',
      "model.call",
    );
    bindSourceSpan(
      this,
      "node.capability",
      '.capabilityInvoke("node.capability"',
      "capability.invoke",
    );
    bindSourceSpan(
      this,
      "node.await",
      '.awaitEvent("node.await"',
      "await.event",
    );
    this.returnRegion("region.return");
    return this;
  }
}

/** Build the TypeScript conversational example. */
export function buildConversational(): ConversationalAgent {
  return new ConversationalAgent({
    program_id: "ConversationalExample",
    input_type_ref: "ConversationInput",
    output_type_ref: "ConversationOutput",
    context_type_ref: "ConversationContext",
  }).defineConversation();
}
