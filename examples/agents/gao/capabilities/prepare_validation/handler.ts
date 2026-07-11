// Builds a typed request for APXM's authoritative validation surfaces.
import { tool } from "@apxm/frontend";

type WorkflowArtifactKind = "air" | "workflow_draft";

interface PrepareValidationArgs {
  workflow_name: string;
  artifact_kind: WorkflowArtifactKind;
  artifact: string;
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
      authoritative_validation: [
        "typescript_conversational_agent_validate",
        "studio_lower",
        "server_compile_or_compose_workflow",
        "runtime_write_boundary",
      ],
      next_action: "ask_user_before_write_or_execute",
    },
    null,
    2,
  );
});
