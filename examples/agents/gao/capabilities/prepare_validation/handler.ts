// Defines Gao's read-only APXM validation-request Tool.
import { Tool } from "@apxm/agent-packaging";

interface PrepareValidationArgs {
  request: string;
  plan: string;
}

/** Prepare a bounded validation request without executing the workflow. */
export const prepareValidation = Tool.define({
  name: "prepare_validation",
  description: "Prepare an APXM validation request for a reviewable workflow plan.",
  input: Tool.object<PrepareValidationArgs>({
    request: Tool.text({ minLength: 1 }),
    plan: Tool.text({ minLength: 1 }),
  }),
  run(args) {
    const validation_request = `Submit this APXM validation request for review: request=${args.request.trim()}; plan=${args.plan.trim()}.`;
    return Tool.answer({ validation_request, next_action: "submit_for_validation" });
  },
});
