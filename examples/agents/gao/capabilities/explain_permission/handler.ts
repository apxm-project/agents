// Explains the approval posture of one capability request.
import { tool } from "@apxm/frontend";

type PermissionClass = "read" | "write" | "execute" | "deploy";

interface ExplainPermissionArgs {
  capability_id: string;
  permission: PermissionClass;
}

/** Explain why a requested capability does or does not require approval. */
export const explainPermission = tool({
  name: "explain_permission",
  description: "Explain the operator approval Gao needs before using an APXM capability.",
  schema: {
    type: "object",
    properties: {
      capability_id: { type: "string" },
      permission: { type: "string", enum: ["read", "write", "execute", "deploy"] },
    },
    required: ["capability_id", "permission"],
    additionalProperties: false,
  },
})((args: ExplainPermissionArgs) => {
  const perm = args.permission;
  const writeLike = perm === "write" || perm === "execute" || perm === "deploy";
  return JSON.stringify(
    {
      capability: args.capability_id,
      permission: perm,
      requires_approval: writeLike,
      reason: writeLike
        ? "This capability can change state or run code, so APXM must ask the operator for an explicit runtime grant."
        : "This is read-only metadata or inspection.",
    },
    null,
    2,
  );
});
