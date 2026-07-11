// Builds a typed request for APXM's authoritative validation surfaces.
import { tool } from "@apxm/frontend";

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

interface PrepareValidationArgs {
  workflow_name: string;
  artifact_kind: WorkflowArtifactKind;
  artifact: string;
}

const VALIDATION_STEPS: readonly ValidationStep[] = [
  { owner: "agents", operation: "validate_air" },
  { owner: "studio", operation: "validate_workflow_draft" },
  { owner: "studio", operation: "lower_workflow_draft" },
  { owner: "runtime", operation: "admit_capabilities" },
  { owner: "runtime", operation: "enforce_write_boundary" },
];

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
    },
    required: ["workflow_name", "artifact_kind", "artifact"],
    additionalProperties: false,
  },
})((args: PrepareValidationArgs) => {
  const preview = args.artifact.slice(0, 2000);
  return JSON.stringify(
    {
      workflow_name: args.workflow_name,
      artifact_kind: args.artifact_kind,
      artifact_preview: preview,
      validation_plan: VALIDATION_STEPS,
      next_action: "ask_user_before_write_or_execute",
    },
    null,
    2,
  );
});
