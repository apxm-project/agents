import { tool } from "@apxm/frontend";

export const explainPermission = tool({
  name: "explain_permission",
  description: "Explain the operator approval Gao needs before using an APXM capability.",
})((capabilityId: unknown, permission: unknown) => {
  const perm = String(permission ?? "");
  const writeLike = perm === "write" || perm === "execute" || perm === "deploy";
  return JSON.stringify(
    {
      capability: String(capabilityId ?? ""),
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
