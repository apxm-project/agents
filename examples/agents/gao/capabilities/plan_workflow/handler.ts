import { tool } from "@apxm/frontend";

export const planWorkflow = tool({
  name: "plan_workflow",
  description:
    "Turn a natural-language request into a structured APXM workflow design plan.",
})((request: unknown) => {
  const plan = {
    request: String(request ?? ""),
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
      reference_examples: ["examples/weather_summary.canvas.json"],
      reference_doc: "skills/workflow-designer/STUDIO_CANVAS.md",
    },
    next_action: "ask_clarifying_question",
  };
  return JSON.stringify(plan, null, 2);
});
