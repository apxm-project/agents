// Converts an operator request into a typed, reviewable workflow plan.
import { tool } from "@apxm/frontend";

interface PlanWorkflowArgs {
  request: string;
}

/** Build a structured workflow design plan without writing artifacts. */
export const planWorkflow = tool({
  name: "plan_workflow",
  description:
    "Turn a natural-language request into a structured APXM workflow design plan.",
  schema: {
    type: "object",
    properties: { request: { type: "string", minLength: 1 } },
    required: ["request"],
  additionalProperties: false,
  },
})((args: PlanWorkflowArgs) => {
  const plan = {
    request: args.request,
    questions: [] as string[],
    workflow: {
      triggers: [] as string[],
      steps: [] as string[],
      capabilities: [] as string[],
    },
    studio_canvas: {
      format: "apxm_studio_workflow",
      tool_node_config: {
        capability: "<capability_id e.g. http_get>",
        args: { url: "https://…" },
      },
      prompt_tokens: "prefer {{node_id}} placeholders; Studio rewrites to label slugs",
      reference_examples: [
        "skills/workflow-designer/examples/capability_pipeline.air",
        "skills/workflow-designer/examples/approval_gate.air",
      ],
      reference_doc: "skills/workflow-designer/STUDIO_CANVAS.md",
    },
    next_action: "ask_clarifying_question",
  };
  return JSON.stringify(plan, null, 2);
});
