// AUTO-GENERATED from openapi-session-v1.yaml. DO NOT EDIT BY HAND.

export type TypedErrorClass = "program_fault" | "server_fault";

export interface TypedError {
  class: TypedErrorClass;
  code: string;
  message: string;
  recovery_hint?: string | null;
}

export interface SessionLedgerView {
  turn_cap?: number | null;
  tool_budgets?: Record<string, number>;
}

export interface SessionStatus {
  session_id: string;
  turn_count: number;
  ledger: SessionLedgerView;
  active_execution_id?: string | null;
}

export type PermissionResponseDecision = "approve" | "deny";

export interface PermissionResponse {
  decision: PermissionResponseDecision;
}

export type SessionRole = "user" | "assistant" | "system";

export interface SessionHistoryMessage {
  role: SessionRole;
  content: string;
}

export interface SessionHistoryResponse {
  messages: SessionHistoryMessage[];
}

export interface CompactSessionResponse {
  ok: boolean;
  session_id: string;
  folded_turns?: number;
}
