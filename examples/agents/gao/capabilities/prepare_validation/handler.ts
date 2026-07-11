// Builds a typed request for APXM's authoritative validation surfaces.
import { tool } from "@apxm/frontend";

import {
  capabilitiesMentionedIn,
  type CapabilityCatalogEntry,
  type PermissionPolicyInput,
  resolveCapability,
} from "../handlers/semantics.js";

type WorkflowArtifactKind = "air" | "workflow_draft";
type ValidationOwner = "agents" | "studio" | "runtime";
type ValidationOperation =
  | "validate_air"
  | "validate_workflow_draft"
  | "lower_workflow_draft"
  | "admit_capabilities"
  | "enforce_write_boundary";

interface ValidationStep {
  owner: ValidationOwner;
  operation: ValidationOperation;
}

interface ValidationIssue {
  capability_id: string;
  kind: "missing_catalog_entry" | "denied_by_policy" | "missing_policy_decision";
}

interface PrepareValidationArgs {
  workflow_name: string;
  artifact_kind: WorkflowArtifactKind;
  artifact: string;
  capability_ids?: string[];
  policy: PermissionPolicyInput;
  catalog: CapabilityCatalogEntry[];
}

/** Prepare a bounded validation request without executing the artifact. */
export const prepareValidation = tool({
  name: "prepare_validation",
  description: "Prepare a structured APXM validation request for a workflow artifact.",
  schema: {
    type: "object",
    properties: {
      workflow_name: { type: "string", minLength: 1 },
      artifact_kind: { type: "string", enum: ["air", "workflow_draft"] },
      artifact: { type: "string", minLength: 1 },
      capability_ids: { type: "array", items: { type: "string", minLength: 1 } },
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
    required: ["workflow_name", "artifact_kind", "artifact", "policy", "catalog"],
    additionalProperties: false,
  },
})((args: PrepareValidationArgs) => {
  const inferredIds = capabilitiesMentionedIn(args.artifact, args.catalog).map((entry) => entry.id);
  const capabilityIds = [...new Set([...(args.capability_ids ?? []), ...inferredIds])];
  const capabilities = capabilityIds.map((id) => resolveCapability(id, args.catalog, args.policy));
  const validationPlan: ValidationStep[] =
    args.artifact_kind === "air"
      ? [{ owner: "agents", operation: "validate_air" }]
      : [
          { owner: "studio", operation: "validate_workflow_draft" },
          { owner: "studio", operation: "lower_workflow_draft" },
        ];
  if (capabilities.length > 0) {
    validationPlan.push({ owner: "runtime", operation: "admit_capabilities" });
  }
  if (capabilities.some((entry) => entry.decision === "ask")) {
    validationPlan.push({ owner: "runtime", operation: "enforce_write_boundary" });
  }

  const issues: ValidationIssue[] = [];
  for (const entry of capabilities) {
    if (!entry.known) {
      issues.push({ capability_id: entry.id, kind: "missing_catalog_entry" });
      continue;
    }
    if (entry.decision === "deny") {
      issues.push({ capability_id: entry.id, kind: "denied_by_policy" });
      continue;
    }
    if (entry.decision === "unknown") {
      issues.push({ capability_id: entry.id, kind: "missing_policy_decision" });
    }
  }

  return {
    workflow_name: args.workflow_name.trim(),
    artifact_kind: args.artifact_kind,
    artifact_preview: args.artifact.slice(0, 2000),
    capabilities: capabilities.map((entry) => ({
      id: entry.id,
      known: entry.known,
      decision: entry.decision,
      requires_approval: entry.decision === "ask",
    })),
    validation_plan: validationPlan,
    issues,
    next_action: issues.length > 0 ? "resolve_validation_issues" : "submit_for_validation",
  };
});
