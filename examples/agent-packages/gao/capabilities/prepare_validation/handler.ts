import { tool } from "@apxm/frontend";

export const prepareValidation = tool({
  name: "prepare_validation",
  description: "Prepare a structured APXM validation request for a workflow artifact.",
})((workflowName: unknown, artifactKind: unknown, artifact: unknown) => {
  const preview = String(artifact ?? "").slice(0, 2000);
  return JSON.stringify(
    {
      workflow_name: String(workflowName ?? ""),
      artifact_kind: String(artifactKind ?? ""),
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
