// Explains the approval posture of one capability request.
import { tool } from "@apxm/agent-packaging";

import {
  type CapabilityCatalogEntry,
  type PermissionPolicyInput,
  resolveCapability,
} from "../handlers/semantics.js";

interface ExplainPermissionArgs {
  capability_id: string;
  policy: PermissionPolicyInput;
  catalog: CapabilityCatalogEntry[];
}

/** Explain why a requested capability does or does not require approval. */
export const explainPermission = tool({
  name: "explain_permission",
  description: "Explain the operator approval Gao needs before using an APXM capability.",
  schema: {
    type: "object",
    properties: {
      capability_id: { type: "string" },
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
    required: ["capability_id", "policy", "catalog"],
    additionalProperties: false,
  },
})((args: ExplainPermissionArgs) => {
  const capability = resolveCapability(args.capability_id.trim(), args.catalog, args.policy);
  const reason = capability.reason ?? (
    !capability.known
      ? "The host capability catalog does not contain this capability."
      : capability.decision === "ask"
        ? "The supplied permission policy requires an explicit operator grant."
        : capability.decision === "deny"
          ? "The supplied permission policy denies this capability."
          : capability.decision === "allow"
            ? "The supplied permission policy allows this capability without an additional grant."
            : "The supplied permission policy does not define a decision for this capability."
  );
  return {
    capability: capability.id,
    known: capability.known,
    read_only: capability.read_only ?? null,
    decision: capability.decision,
    requires_approval:
      capability.decision === "unknown" ? null : capability.decision === "ask",
    reason,
  };
});
