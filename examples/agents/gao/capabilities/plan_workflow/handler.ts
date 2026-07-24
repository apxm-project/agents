// Defines Gao's read-only APXM workflow-planning Tool.
import { Tool } from "@apxm/agent-packaging";

interface PlanWorkflowArgs {
  request: string;
  catalog: string;
}

/** Build a short workflow plan for review without writing artifacts. */
export const planWorkflow = Tool.define({
  name: "plan_workflow",
  description: "Turn an APXM request and capability summary into a reviewable workflow plan.",
  input: Tool.object<PlanWorkflowArgs>({
    request: Tool.text({ minLength: 1 }),
    catalog: Tool.text({ minLength: 1 }),
  }),
  run(args) {
    const plan = `Review this APXM workflow plan before applying it: request=${args.request.trim()}; capabilities=${args.catalog.trim()}.`;
    return Tool.answer({ plan, next_action: "review_plan" });
  },
});
