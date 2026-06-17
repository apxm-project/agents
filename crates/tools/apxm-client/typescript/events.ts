// SSE permission prompt helpers (mirrors apxm-client Rust events.rs).

import type { PermissionResponseDecision } from "./types";

export interface ApprovalPrompt {
  approval_id: string;
  tool_name: string;
  agent_code: string;
  risk_level: string;
}

export function eventKind(json: Record<string, unknown>): string | undefined {
  const kind = json.kind;
  if (typeof kind === "string") return kind;
  const payload = json.payload;
  if (payload && typeof payload === "object") {
    const pk = (payload as Record<string, unknown>).kind;
    if (typeof pk === "string") return pk;
  }
  return undefined;
}

/** Detect an `approval_request` frame and return prompt fields. */
export function parseApprovalPrompt(json: Record<string, unknown>): ApprovalPrompt | null {
  if (eventKind(json) !== "approval_request") return null;
  const payload = (json.payload ?? json) as Record<string, unknown>;
  const approvalId = payload.approval_id;
  const toolName = payload.tool_name;
  if (typeof approvalId !== "string" || typeof toolName !== "string") return null;
  return {
    approval_id: approvalId,
    tool_name: toolName,
    agent_code: typeof payload.agent_code === "string" ? payload.agent_code : "",
    risk_level: typeof payload.risk_level === "string" ? payload.risk_level : "medium",
  };
}

export function decisionForGrant(approve: boolean, forSession = false): PermissionResponseDecision {
  if (!approve) return "deny";
  return forSession ? "approve_for_session" : "approve";
}
