// Converts an operator request into a typed, reviewable workflow plan.
import { tool } from "@apxm/frontend";

import {
  capabilitiesMentionedIn,
  type CapabilityCatalogEntry,
  type PermissionPolicyInput,
  resolveCapability,
  splitWorkflowRequest,
} from "../handlers/semantics.js";

interface PlanWorkflowArgs {
  request: string;
  policy: PermissionPolicyInput;
  catalog: CapabilityCatalogEntry[];
}

/** Build a structured workflow design plan without writing artifacts. */
export const planWorkflow = tool({
  name: "plan_workflow",
  description:
    "Turn a natural-language request into a structured APXM workflow design plan.",
  schema: {
    type: "object",
    properties: {
      request: { type: "string", minLength: 1 },
      policy: {
        type: "object",
        properties: {
          default_decision: { type: "string", enum: ["allow", "ask", "deny"] },
          entries: {
            type: "array",
            items: {
              type: "object",
              properties: {
                capability_id: { type: "string", minLength: 1 },
                decision: { type: "string", enum: ["allow", "ask", "deny"] },
                reason: { type: "string" },
              },
              required: ["capability_id", "decision"],
              additionalProperties: false,
            },
          },
        },
        additionalProperties: false,
      },
      catalog: {
        type: "array",
        items: {
          type: "object",
          properties: {
            id: { type: "string", minLength: 1 },
            description: { type: "string" },
            read_only: { type: "boolean" },
            permission: { type: "string", enum: ["allow", "ask", "deny"] },
          },
          required: ["id"],
          additionalProperties: false,
        },
      },
    },
    required: ["request", "policy", "catalog"],
    additionalProperties: false,
  },
})((args: PlanWorkflowArgs) => {
  const request = args.request.trim();
  const { triggers, actions } = splitWorkflowRequest(request);
  const mentioned = capabilitiesMentionedIn(request, args.catalog);
  const capabilityPlans = mentioned.map((entry) => resolveCapability(entry.id, args.catalog, args.policy));
  const questions: string[] = [];
  if (triggers.length === 0) {
    questions.push("What event or schedule should trigger this workflow?");
  }
  if (args.catalog.length === 0) {
    questions.push("Which host-supplied capabilities are available for this workflow?");
  } else if (mentioned.length === 0) {
    questions.push("Which catalog capabilities should implement the requested actions?");
  }
  const denied = capabilityPlans.filter((entry) => entry.decision === "deny");
  const unknown = capabilityPlans.filter((entry) => entry.decision === "unknown");
  if (unknown.length > 0) {
    questions.push(`What permission policy applies to: ${unknown.map((entry) => entry.id).join(", ")}?`);
  }

  return {
    request: args.request,
    questions,
    workflow: {
      triggers,
      steps: actions.map((instruction, index) => {
        const capability = capabilitiesMentionedIn(instruction, mentioned)[0];
        return {
          id: `step_${index + 1}`,
          instruction,
          ...(capability ? { capability_id: capability.id } : {}),
        };
      }),
      capabilities: capabilityPlans.map((entry) => ({
        id: entry.id,
        decision: entry.decision,
        requires_approval: entry.decision === "ask",
        ...(entry.reason ? { reason: entry.reason } : {}),
      })),
    },
    next_action:
      denied.length > 0
        ? "revise_request_or_policy"
        : questions.length > 0
          ? "ask_clarifying_question"
          : "review_plan",
  };
});
