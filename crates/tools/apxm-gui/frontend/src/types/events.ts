import * as EK from "@/lib/event-kinds";

export type TraceEvent = {
  timestamp: string;
  kind: string;
  node_id?: number;
  [key: string]: unknown;
};

export type AgentStreamEvent =
  | { kind: typeof EK.TOKEN.name; token?: string; text?: string }
  | {
      kind: typeof EK.TOOL_CALL.name;
      id: string;
      name: string;
      arguments?: Record<string, unknown>;
    }
  | {
      kind: typeof EK.TOOL_RESULT.name;
      id: string;
      success: boolean;
      output?: string;
    }
  | {
      kind: typeof EK.USAGE.name;
      inputTokens?: number;
      outputTokens?: number;
      input_tokens?: number;
      output_tokens?: number;
    }
  | {
      kind: typeof EK.DONE.name;
      sessionId?: string;
      session_id?: string;
      stopReason?: string;
    }
  | { kind: typeof EK.ERROR.name; error?: string; message?: string }
  | { kind: "unknown"; rawKind: string; token?: string; error?: string };

export type EventMeta = {
  seq: number;
  timestamp: string;
  trace_id: string;
  source: string;
  span_id: string;
  parent_span_id: string | null;
  scope_id?: string | null;
};

export type EventPayload =
  | { kind: typeof EK.TOKEN.name; text: string; node_id: number; span_id: string; parent_span_id: string | null }
  | {
      kind: typeof EK.OPERATION_START.name;
      node_id: number;
      op_type: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | {
      kind: "operation_complete";
      node_id: number;
      op_type: string;
      duration_ms: number;
      span_id: string;
      parent_span_id: string | null;
    }
  | { kind: "operation_error"; node_id: number; error: string; span_id: string; parent_span_id: string | null }
  | { kind: typeof EK.SESSION_START.name; session_id: string; span_id: string; parent_span_id: string | null }
  | { kind: "session_complete"; session_id: string; duration_ms: number; span_id: string; parent_span_id: string | null }
  | { kind: "session_error"; session_id: string; error: string; span_id: string; parent_span_id: string | null }
  | {
      kind: typeof EK.SCHEDULER_DECISION.name;
      node_id: number;
      delay_ms: number;
      reason: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | {
      kind: typeof EK.HEAD_OF_LINE_BLOCK.name;
      blocker_node: number;
      blocked_node: number;
      wait_ms: number;
      reason: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | {
      kind: typeof EK.MEMORY_READ.name;
      node_id?: number;
      scope?: string;
      tier?: string;
      key: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | {
      kind: typeof EK.MEMORY_WRITE.name;
      node_id?: number;
      scope?: string;
      tier?: string;
      key: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | { kind: "spawn_agent"; node_id: number; agent_type: string; span_id: string; parent_span_id: string | null }
  | { kind: "agent_complete"; node_id: number; agent_type: string; span_id: string; parent_span_id: string | null }
  | {
      kind: typeof EK.CHECKPOINT_SAVED.name;
      node_id: number;
      checkpoint_id: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | {
      kind: typeof EK.CHECKPOINT_RESTORED.name;
      checkpoint_id: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | { kind: "capability_invoked"; node_id: number; capability: string; span_id: string; parent_span_id: string | null }
  | {
      kind: typeof EK.RETRY.name;
      node_id: number;
      attempt: number;
      reason: string;
      span_id: string;
      parent_span_id: string | null;
    }
  | { kind: string; span_id?: string; parent_span_id?: string | null; [key: string]: unknown };

export type NodeLiveStatus = "pending" | "running" | "completed" | "failed";
